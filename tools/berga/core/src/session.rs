//! Blocking driver of the sans-IO [`Client`] over a [`Transport`]: what the CLI and the desktop app use.

use crate::client::{Client, Incoming};
use crate::proto::{CHUNK, REBOOT_MODES, op};
use crate::server::{PULL_ROOTS, PUSH_ROOTS, path_allowed, run_allowed};
use crate::transport::Transport;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct Reply {
    pub head: Value,
    pub body: Vec<u8>,
}

pub struct Session<T: Transport> {
    pub transport: T,
    client: Client,
    start: Instant,
    /// Events and late replies that arrived while waiting for something else.
    queued: VecDeque<Incoming>,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T) -> Session<T> {
        Session { transport, client: Client::new(), start: Instant::now(), queued: VecDeque::new() }
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// One request, waiting for its reply (at most 15 s). Events seen meanwhile are kept for [`Self::next_event`].
    pub fn call(&mut self, op: u16, head: Value, body: Vec<u8>) -> Result<Reply> {
        let (tag, bytes) = self.client.request(op, head, body, self.now())?;
        self.transport.send(&bytes)?;
        loop {
            for inc in self.client.expire(self.now()) {
                if matches!(inc, Incoming::TimedOut { tag: t, .. } if t == tag) {
                    return Err(Error::Timeout(format!("{} (no answer from the phone in {} s)", op::name(op), self.client.timeout_ms / 1000)));
                }
            }
            let Some(data) = self.transport.recv(Duration::from_millis(250))? else { continue };
            for inc in self.client.feed(&data) {
                match inc {
                    Incoming::Reply { tag: t, head, body, .. } if t == tag => return Ok(Reply { head, body }),
                    Incoming::Failed { tag: t, error, .. } if t == tag => return Err(Error::Device(error)),
                    Incoming::Protocol(e) => return Err(Error::Device(e)),
                    other => self.queued.push_back(other),
                }
            }
        }
    }

    /// The next event (log line); `None` after `timeout` without one.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<Incoming>> {
        if let Some(i) = self.queued.pop_front() {
            return Ok(Some(i));
        }
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(data) = self.transport.recv(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(250)))? {
                self.queued.extend(self.client.feed(&data));
                if let Some(i) = self.queued.pop_front() {
                    return Ok(Some(i));
                }
            }
        }
        Ok(None)
    }

    pub fn hello(&mut self) -> Result<Value> {
        Ok(self.call(op::HELLO, Value::Null, vec![])?.head)
    }
    pub fn status(&mut self) -> Result<Value> {
        Ok(self.call(op::STATUS, Value::Null, vec![])?.head)
    }
    pub fn metrics(&mut self) -> Result<Value> {
        Ok(self.call(op::METRICS, Value::Null, vec![])?.head)
    }
    pub fn reboot(&mut self, mode: &str) -> Result<()> {
        if !REBOOT_MODES.contains(&mode) {
            return Err(Error::Refused(format!("reboot mode {mode}: one of {}", REBOOT_MODES.join(", "))));
        }
        self.call(op::REBOOT, Value::Null, mode.as_bytes().to_vec()).map(|_| ())
    }
    /// Starts the kernel log stream with `backlog` bytes of history; the text arrives as [`Incoming::Event`]s
    /// ([`Self::next_event`]). A new HELLO stops it (a new client).
    pub fn log_start(&mut self, backlog: u32) -> Result<()> {
        self.call(op::LOG_START, Value::Null, backlog.to_string().into_bytes()).map(|_| ())
    }
    pub fn log_stop(&mut self) -> Result<()> {
        self.call(op::LOG_STOP, Value::Null, vec![]).map(|_| ())
    }
    pub fn run(&mut self, cmd: &str, args: &[String]) -> Result<(i32, Vec<u8>)> {
        run_allowed(cmd, args).map_err(Error::Refused)?;
        let r = self.call(op::RUN, json!({ "cmd": cmd, "args": args }), vec![])?;
        Ok((r.head["code"].as_i64().unwrap_or(-1) as i32, r.body))
    }
    pub fn screenshot(&mut self) -> Result<Vec<u8>> {
        Ok(self.call(op::SCREENSHOT, Value::Null, vec![])?.body)
    }

    /// Writes `data` to `path` on the phone in [`CHUNK`]-sized requests; `progress(done, total)`.
    pub fn push(&mut self, path: &str, data: &[u8], mode: u32, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
        path_allowed(path, &PUSH_ROOTS).map_err(Error::Refused)?;
        let total = data.len() as u64;
        let mut off = 0usize;
        loop {
            let end = (off + CHUNK).min(data.len());
            self.call(op::PUSH, json!({ "path": path, "offset": off, "truncate": off == 0, "mode": mode }), data[off..end].to_vec())?;
            off = end;
            progress(off as u64, total);
            if off >= data.len() {
                return Ok(());
            }
        }
    }

    pub fn pull(&mut self, path: &str, progress: &mut dyn FnMut(u64, u64)) -> Result<Vec<u8>> {
        path_allowed(path, &PULL_ROOTS).map_err(Error::Refused)?;
        let mut out = Vec::new();
        loop {
            let r = self.call(op::PULL, json!({ "path": path, "offset": out.len(), "len": CHUNK }), vec![])?;
            out.extend_from_slice(&r.body);
            let size = r.head["size"].as_u64().unwrap_or(0);
            progress(out.len() as u64, size);
            if r.head["eof"].as_bool().unwrap_or(true) || r.body.is_empty() {
                return Ok(out);
            }
        }
    }

    /// Pushes a picture/video to the staging folder and moves it into the camera roll.
    pub fn upload_media(&mut self, name: &str, data: &[u8], progress: &mut dyn FnMut(u64, u64)) -> Result<Value> {
        let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect();
        let staged = format!("/var/mobile/Media/berga/{}", safe.trim_start_matches('.'));
        self.push(&staged, data, 0o644, progress)?;
        Ok(self.call(op::MEDIA_IMPORT, json!({ "path": staged }), vec![])?.head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeTransport;

    #[test]
    fn session_against_fake() {
        let mut s = Session::new(FakeTransport::default());
        assert_eq!(s.hello().unwrap()["proto"], 1);
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 7) as u8).collect();
        let mut seen = 0;
        s.push("/var/root/berga/blob", &data, 0o644, &mut |d, _| seen = d).unwrap();
        assert_eq!(seen, data.len() as u64);
        assert_eq!(s.pull("/var/root/berga/blob", &mut |_, _| {}).unwrap(), data);
        assert!(matches!(s.reboot("edl"), Err(Error::Refused(_))));
        assert!(matches!(s.push("/System/x", b"x", 0o644, &mut |_, _| {}), Err(Error::Refused(_))));
        let (code, out) = s.run("uptime", &[]).unwrap();
        assert_eq!(code, 0);
        assert!(String::from_utf8_lossy(&out).contains("up"));
        let r = s.upload_media("My Photo.jpg", b"jpeg", &mut |_, _| {}).unwrap();
        assert!(r["path"].as_str().unwrap().starts_with("/var/mobile/Media/DCIM/"));
    }
}
