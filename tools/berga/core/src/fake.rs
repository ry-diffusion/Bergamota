//! A fake phone that speaks the whole berga protocol: the kernel ops (hello, reboot, log stream) and the daemon ops
//! through [`crate::server::handle`] with an in-memory backend. Used by the tests, `berga --fake` and the web demo
//! (the wasm build runs it in the page), so every UI can be exercised without the phone.

use crate::proto::{Decoder, Frame, Kind, op, REBOOT_MODES, FLAG_MORE, CHUNK};
use crate::server::{self, Backend};
use crate::transport::Transport;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

pub struct FakeBackend {
    pub files: BTreeMap<String, Vec<u8>>,
    pub camera_roll: Vec<String>,
    pub now_ms: u64,
}

impl FakeBackend {
    fn new() -> FakeBackend {
        FakeBackend { files: BTreeMap::new(), camera_roll: Vec::new(), now_ms: 0 }
    }
}

impl Backend for FakeBackend {
    fn status(&mut self) -> Result<Value, String> {
        Ok(json!({
            "fake": true,
            "ios": { "version": "18.7.8", "build": "22H352", "device": "iPhone11,8" },
            "kernel": "Darwin Kernel Version 24.6.0 (fake): xnu-11215.81.4~bergamota/RELEASE_ARM64_BERGAMOTA",
            "uptime_s": 120 + self.now_ms / 1000,
            "disk": { "total": 51_539_607_552u64, "free": 9_663_676_416u64 + self.files.values().map(|f| f.len() as u64).sum::<u64>() },
            "battery": { "percent": 81, "charging": true },
            "usb": { "berga": true, "ecm": true },
        }))
    }

    fn metrics(&mut self) -> Result<Value, String> {
        let t = (self.now_ms / 1000) as f64;
        let wave = |p: f64, lo: f64, hi: f64| lo + (hi - lo) * (0.5 + 0.5 * (t / p).sin());
        Ok(json!({
            "schema": 1,
            "fetched": "fake",
            "cpu": {
                "clusters": [
                    { "name": "LITTLE", "cpus": [0,1,2,3], "curKhz": 1_766_400, "loadPct": wave(7.0, 5.0, 40.0), "mode": "static", "capPercent": 100 },
                    { "name": "big", "cpus": [4,5,6,7], "curKhz": 2_803_200, "loadPct": wave(5.0, 0.0, 70.0), "mode": "governor", "governor": "schedutil", "minKhz": 825_600, "maxKhz": 2_803_200 }
                ],
                "loadPct": [wave(3.0,0.,50.), wave(4.0,0.,40.), wave(5.0,0.,30.), wave(6.0,0.,20.), wave(2.0,0.,90.), wave(3.0,0.,60.), 3.0, 1.0]
            },
            "gpu": { "state": "active", "level": 2, "levels": 7, "mhz": 520, "busyPct": wave(4.0, 10.0, 80.0), "clients": 2 },
            "renderer": { "label": "Crimson (Turnip)", "requested": "crimson", "requestedBy": "boot-arg", "injected": true, "gpuClientOpen": true },
            "battery": { "present": true, "percent": 81, "charging": true, "currentMa": 640.0, "voltageMv": 4180.0 },
            "thermal": { "batteryC": wave(30.0, 31.0, 35.0) },
            "errors": []
        }))
    }

    fn run(&mut self, path: &str, args: &[String]) -> Result<(i32, Vec<u8>), String> {
        let out = match path.rsplit('/').next().unwrap_or("") {
            "uptime" => format!("12:00  up {} mins, 1 user, load averages: 1.20 0.90 0.70\n", 2 + self.now_ms / 60000),
            "sysctl" => args.iter().map(|a| format!("{a}: (fake)\n")).collect(),
            "bergamotametrics" => serde_json::to_string_pretty(&self.metrics()?).unwrap() + "\n",
            "bergamotauicache" => format!("registered {}\n", args.join(" ")),
            other => format!("{other} {} (fake)\n", args.join(" ")),
        };
        Ok((0, out.into_bytes()))
    }

    fn write_file(&mut self, path: &str, offset: u64, truncate: bool, _mode: u32, data: &[u8]) -> Result<(), String> {
        let f = self.files.entry(path.to_string()).or_default();
        if truncate {
            f.clear();
        }
        let off = offset as usize;
        if off > f.len() {
            return Err(format!("{path}: write at {off} past the end ({})", f.len()));
        }
        f.truncate(off);
        f.extend_from_slice(data);
        Ok(())
    }

    fn read_file(&mut self, path: &str, offset: u64, len: usize) -> Result<(u64, Vec<u8>), String> {
        let f = self.files.get(path).ok_or_else(|| format!("{path}: No such file or directory"))?;
        let off = (offset as usize).min(f.len());
        Ok((f.len() as u64, f[off..(off + len).min(f.len())].to_vec()))
    }

    fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        Ok(demo_png(self.now_ms))
    }

    fn import_media(&mut self, path: &str) -> Result<Value, String> {
        let data = self.files.remove(path).ok_or_else(|| format!("{path}: No such file or directory"))?;
        let ext = path.rsplit('.').next().unwrap_or("JPG").to_ascii_uppercase();
        let dest = format!("/var/mobile/Media/DCIM/100APPLE/IMG_{:04}.{ext}", 1 + self.camera_roll.len());
        self.files.insert(dest.clone(), data);
        self.camera_roll.push(dest.clone());
        Ok(json!({ "path": dest }))
    }
}

/// The fake phone, sans-IO: [`FakePhone::feed`] host bytes in, [`FakePhone::take`] frames out (one per IN transfer).
pub struct FakePhone {
    pub backend: FakeBackend,
    dec: Decoder,
    out: VecDeque<Vec<u8>>,
    log: Option<u32>,
    log_seq: u64,
    last_tick: u64,
    /// The last REBOOT mode received (the fake keeps running).
    pub rebooted: Option<String>,
    /// Pretend the daemon is not running: daemon ops get the kernel's "no daemon" error.
    pub daemon: bool,
}

impl Default for FakePhone {
    fn default() -> Self {
        FakePhone::new()
    }
}

impl FakePhone {
    pub fn new() -> FakePhone {
        FakePhone { backend: FakeBackend::new(), dec: Decoder::new(), out: VecDeque::new(), log: None, log_seq: 0, last_tick: 0, rebooted: None, daemon: true }
    }

    fn send(&mut self, f: Frame) {
        // frames the fake builds always fit; an encode error here is a bug in the fake
        self.out.push_back(f.encode().expect("fake frame"));
    }

    fn log_line(&mut self, text: &str) {
        if let Some(tag) = self.log {
            let mut f = Frame::new(Kind::Event, op::LOG_START, tag, Value::Null, text.as_bytes().to_vec());
            f.flags = FLAG_MORE;
            self.send(f);
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.dec.push(bytes);
        loop {
            let f = match self.dec.next() {
                Ok(Some(f)) => f,
                Ok(None) => return,
                // the kernel drops a corrupt stream the same way and answers nothing
                Err(_) => return,
            };
            self.request(f);
        }
    }

    fn request(&mut self, f: Frame) {
        let reply = |head: Value| Frame::new(Kind::Response, f.op, f.tag, head, vec![]);
        match f.op {
            op::HELLO => self.send(reply(json!({
                "proto": 1, "kernel": "Darwin Kernel Version 24.6.0 (fake)", "series": "11215",
                "uptime_ms": 120_000 + self.backend.now_ms, "daemon": self.daemon, "max_frame": crate::proto::MAX_FRAME, "fake": true
            }))),
            op::REBOOT => {
                let mode = String::from_utf8_lossy(&f.body).to_string();
                if REBOOT_MODES.contains(&mode.as_str()) {
                    self.send(reply(json!({ "mode": mode })));
                    self.log_line(&format!("BERGAMOTA REBOOT: berga USB request: {mode} (fake: nothing happens)\n"));
                    self.rebooted = Some(mode);
                } else {
                    self.send(Frame::error(f.op, f.tag, "unknown reboot mode"));
                }
            }
            op::LOG_START => {
                self.log = Some(f.tag);
                self.send(reply(json!({ "position": self.log_seq })));
                for l in ["BERGAMOTA USB: berga interface configured (fake)\n", "launchd: system bootstrap (fake backlog)\n"] {
                    self.log_line(l);
                }
            }
            op::LOG_STOP => {
                self.log = None;
                self.send(reply(Value::Null));
            }
            o if op::is_kernel(o) => self.send(Frame::error(o, f.tag, "unknown kernel op")),
            _ if !self.daemon => self.send(Frame::error(f.op, f.tag, "no daemon: bergad is not running on the phone")),
            _ => {
                let r = server::handle(&mut self.backend, &f);
                self.send(r);
            }
        }
    }

    /// Time passes: one log line per second while the log stream is on.
    pub fn tick(&mut self, now_ms: u64) {
        self.backend.now_ms = now_ms;
        while self.last_tick + 1000 <= now_ms {
            self.last_tick += 1000;
            self.log_seq += 1;
            let line = match self.log_seq % 4 {
                0 => format!("BERGAMOTA FB: {} frames, 30 Hz, vsync ok\n", self.log_seq * 30),
                1 => "SpringBoard[52]: [fake] heartbeat\n".to_string(),
                2 => format!("BERGAMOTA USB: ue0 rx {} tx {} frames\n", self.log_seq * 7, self.log_seq * 5),
                _ => "backboardd[48]: [fake] HID event\n".to_string(),
            };
            self.log_line(&line);
        }
    }

    pub fn take(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }
}

/// [`FakePhone`] as a blocking [`Transport`] with real time (for `berga --fake`).
pub struct FakeTransport {
    pub phone: FakePhone,
    start: Instant,
}

impl Default for FakeTransport {
    fn default() -> Self {
        FakeTransport { phone: FakePhone::new(), start: Instant::now() }
    }
}

impl Transport for FakeTransport {
    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.len() > crate::proto::MAX_FRAME {
            return Err(std::io::Error::other("transfer larger than the phone's buffer"));
        }
        self.phone.feed(bytes);
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        loop {
            self.phone.tick(self.start.elapsed().as_millis() as u64);
            if let Some(b) = self.phone.take() {
                return Ok(Some(b));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
        }
    }
}

/// A small PNG (stored deflate, no compression library): a gradient with a moving bar, like a screen.
pub fn demo_png(now_ms: u64) -> Vec<u8> {
    let (w, h) = (90u32, 195u32);
    let bar = ((now_ms / 100) % h as u64) as u32;
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let on = y.abs_diff(bar) < 4;
            raw.extend_from_slice(&[if on { 220 } else { (20 + y / 2) as u8 }, if on { 240 } else { (30 + x) as u8 }, 60]);
        }
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |ty: &[u8], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(ty);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    debug_assert!(png.len() <= CHUNK);
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Client, Incoming};

    fn roundtrip(c: &mut Client, p: &mut FakePhone, op: u16, head: Value, body: Vec<u8>) -> Incoming {
        let (tag, bytes) = c.request(op, head, body, 0).unwrap();
        p.feed(&bytes);
        while let Some(b) = p.take() {
            for i in c.feed(&b) {
                match &i {
                    Incoming::Reply { tag: t, .. } | Incoming::Failed { tag: t, .. } if *t == tag => return i,
                    _ => {}
                }
            }
        }
        panic!("no reply");
    }

    #[test]
    fn kernel_and_daemon_ops() {
        let mut c = Client::new();
        let mut p = FakePhone::new();
        assert!(matches!(roundtrip(&mut c, &mut p, op::HELLO, Value::Null, vec![]), Incoming::Reply { head, .. } if head["proto"] == 1));
        assert!(matches!(roundtrip(&mut c, &mut p, op::REBOOT, Value::Null, b"fastboot".to_vec()), Incoming::Reply { .. }));
        assert_eq!(p.rebooted.as_deref(), Some("fastboot"));
        assert!(matches!(roundtrip(&mut c, &mut p, op::REBOOT, Value::Null, b"edl".to_vec()), Incoming::Failed { .. }));
        // push two chunks, pull back, import into the camera roll
        let path = "/var/mobile/Media/berga/a.jpg";
        roundtrip(&mut c, &mut p, op::PUSH, json!({"path": path, "offset": 0, "truncate": true}), b"hello ".to_vec());
        roundtrip(&mut c, &mut p, op::PUSH, json!({"path": path, "offset": 6}), b"world".to_vec());
        match roundtrip(&mut c, &mut p, op::PULL, json!({"path": path, "offset": 0, "len": 100}), vec![]) {
            Incoming::Reply { head, body, .. } => {
                assert_eq!(body, b"hello world");
                assert_eq!(head["eof"], true);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(roundtrip(&mut c, &mut p, op::MEDIA_IMPORT, json!({"path": path}), vec![]),
            Incoming::Reply { head, .. } if head["path"] == "/var/mobile/Media/DCIM/100APPLE/IMG_0001.JPG"));
        assert!(matches!(roundtrip(&mut c, &mut p, op::PUSH, json!({"path": "/etc/passwd"}), b"x".to_vec()), Incoming::Failed { .. }));
        assert!(matches!(roundtrip(&mut c, &mut p, op::RUN, json!({"cmd": "sh"}), vec![]), Incoming::Failed { .. }));
        match roundtrip(&mut c, &mut p, op::SCREENSHOT, Value::Null, vec![]) {
            Incoming::Reply { body, .. } => assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n"),
            other => panic!("{other:?}"),
        }
        p.daemon = false;
        assert!(matches!(roundtrip(&mut c, &mut p, op::STATUS, Value::Null, vec![]), Incoming::Failed { error, .. } if error.contains("no daemon")));
    }

    #[test]
    fn log_stream() {
        let mut c = Client::new();
        let mut p = FakePhone::new();
        let (tag, b) = c.request(op::LOG_START, Value::Null, vec![], 0).unwrap();
        p.feed(&b);
        p.tick(3500);
        let mut events = 0;
        while let Some(b) = p.take() {
            events += c.feed(&b).iter().filter(|i| matches!(i, Incoming::Event { tag: t, .. } if *t == tag)).count();
        }
        assert_eq!(events, 2 + 3);
    }

    #[test]
    fn png_is_valid() {
        let png = demo_png(0);
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert!(png.len() < CHUNK);
    }
}
