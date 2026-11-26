//! Fastboot (ABL and Enxerto's Bergamota fastboot): ASCII commands of at most 64 bytes on bulk OUT; replies on
//! bulk IN are INFO/TEXT (keep reading), DATA%08x (send that many bytes), OKAY or FAIL.
//! The sans-IO half ([`command`], [`parse_reply`]) is what the browser uses too; [`Fastboot`] drives it natively.
//! Only allowlisted commands can be built: no erase, format, flashing unlock, set_active or free-form oem.

use crate::partition;
use crate::sparse::{self, Source};
use crate::transport::Transport;
use crate::{Error, Result};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Okay(String),
    Fail(String),
    Data(u32),
    Info(String),
}

/// Commands berga may send. `flash:` and `oem hash:` only for boot/userdata/logo.
pub fn allowed(cmd: &str) -> bool {
    let hexish = |s: &str| !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some(v) = cmd.strip_prefix("getvar:") {
        return !v.is_empty() && v.len() <= 58 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"_:.-".contains(&b));
    }
    if let Some(n) = cmd.strip_prefix("download:") {
        return n.len() == 8 && hexish(n);
    }
    if let Some(p) = cmd.strip_prefix("flash:") {
        return partition::check(p).is_ok();
    }
    if let Some(a) = cmd.strip_prefix("oem hash:") {
        // oem hash:<part>:<off>:<len>:<bs> (hex), Enxerto only
        let f: Vec<&str> = a.split(':').collect();
        return f.len() == 4 && partition::check(f[0]).is_ok() && f[1..].iter().all(|x| hexish(x));
    }
    matches!(cmd, "boot" | "reboot" | "reboot-bootloader" | "upload" | "continue")
}

/// Bytes of one command, refusing anything not allowlisted.
pub fn command(cmd: &str) -> Result<Vec<u8>> {
    if cmd.len() > 64 || !allowed(cmd) {
        return Err(Error::Refused(format!("fastboot command {cmd:?} is not allowed")));
    }
    Ok(cmd.as_bytes().to_vec())
}

pub fn parse_reply(b: &[u8]) -> Result<Reply> {
    if b.len() < 4 {
        return Err(Error::Device(format!("short fastboot reply {:?}", String::from_utf8_lossy(b))));
    }
    let text = String::from_utf8_lossy(&b[4..]).trim_end_matches('\0').to_string();
    Ok(match &b[..4] {
        b"OKAY" => Reply::Okay(text),
        b"FAIL" => Reply::Fail(text),
        b"INFO" | b"TEXT" => Reply::Info(text),
        b"DATA" => Reply::Data(u32::from_str_radix(text.trim(), 16).map_err(|_| Error::Device(format!("bad DATA reply {text:?}")))?),
        _ => return Err(Error::Device(format!("unexpected fastboot reply {:?}", String::from_utf8_lossy(b)))),
    })
}

/// Progress of a long operation.
#[derive(Clone, Debug)]
pub enum Progress {
    Info(String),
    /// Bytes of piece `piece` (1-based, of `pieces`) sent so far.
    Sent { piece: usize, pieces: usize, done: u64, total: u64 },
    Flashing { piece: usize, pieces: usize },
}

/// Producer of a download's bytes: calls its sink with the data, in order.
pub type Produce<'a> = dyn FnMut(&mut dyn FnMut(&[u8]) -> Result<()>) -> Result<()> + 'a;

pub struct Fastboot<T: Transport> {
    pub t: T,
}

impl<T: Transport> Fastboot<T> {
    pub fn new(t: T) -> Fastboot<T> {
        Fastboot { t }
    }

    fn read_reply(&mut self, timeout: Duration, info: &mut dyn FnMut(Progress)) -> Result<Reply> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout(format!("fastboot: no answer in {} s", timeout.as_secs())));
            }
            let Some(b) = self.t.recv(left.min(Duration::from_secs(1)))? else { continue };
            match parse_reply(&b)? {
                Reply::Info(s) => info(Progress::Info(s)),
                r => return Ok(r),
            }
        }
    }

    /// Sends `cmd`, returns OKAY's text or DATA's size; FAIL becomes an error.
    pub fn run(&mut self, cmd: &str, timeout: Duration, info: &mut dyn FnMut(Progress)) -> Result<Reply> {
        self.t.send(&command(cmd)?)?;
        match self.read_reply(timeout, info)? {
            Reply::Fail(why) => Err(Error::Device(format!("{cmd}: FAIL {why}"))),
            r => Ok(r),
        }
    }

    pub fn getvar(&mut self, name: &str) -> Result<String> {
        match self.run(&format!("getvar:{name}"), Duration::from_secs(10), &mut |_| {})? {
            Reply::Okay(v) => Ok(v.trim().to_string()),
            r => Err(Error::Device(format!("getvar {name}: unexpected {r:?}"))),
        }
    }

    /// max-download-size, capped at 512 MiB like the app does (`fastboot -S 512M`).
    pub fn max_download(&mut self) -> Result<u64> {
        let v = self.getvar("max-download-size")?;
        let n = u64::from_str_radix(v.trim_start_matches("0x"), 16).or_else(|_| v.parse()).map_err(|_| Error::Device(format!("max-download-size {v:?}")))?;
        Ok(n.min(512 << 20))
    }

    /// `download:` + exactly `size` bytes produced by `produce`, which calls its sink with data in order.
    pub fn download(&mut self, size: u64, produce: &mut Produce, on_bytes: &mut dyn FnMut(u64)) -> Result<()> {
        if size == 0 || size > 0xffff_ffff {
            return Err(Error::Refused(format!("bad download size {size}")));
        }
        match self.run(&format!("download:{size:08x}"), Duration::from_secs(15), &mut |_| {})? {
            Reply::Data(n) if n as u64 == size => {}
            r => return Err(Error::Device(format!("download not accepted: {r:?}"))),
        }
        let mut sent = 0u64;
        let t = &mut self.t;
        produce(&mut |b: &[u8]| {
            if sent + b.len() as u64 > size {
                return Err(Error::Refused("image produced more bytes than announced".into()));
            }
            t.send(b)?;
            sent += b.len() as u64;
            on_bytes(sent);
            Ok(())
        })?;
        if sent != size {
            return Err(Error::Refused(format!("image produced {sent} bytes instead of {size}")));
        }
        match self.read_reply(Duration::from_secs(60), &mut |_| {})? {
            Reply::Okay(_) => Ok(()),
            r => Err(Error::Device(format!("download failed: {r:?}"))),
        }
    }

    /// `fastboot boot IMG`: download + boot.
    pub fn boot(&mut self, image: &[u8], progress: &mut dyn FnMut(Progress)) -> Result<()> {
        if !image.starts_with(b"ANDROID!") {
            return Err(Error::Refused("not an Android boot image".into()));
        }
        let total = image.len() as u64;
        self.download(total, &mut |sink| image.chunks(1 << 20).try_for_each(&mut *sink), &mut |d| progress(Progress::Sent { piece: 1, pieces: 1, done: d, total }))?;
        self.run("boot", Duration::from_secs(15), &mut |p| progress(p)).map(|_| ())
    }

    /// Full flash of `partition` (boot/userdata/logo only): sparse pieces that fit the download buffer.
    pub fn flash(&mut self, partition: &str, src: &mut dyn Source, progress: &mut dyn FnMut(Progress)) -> Result<()> {
        let part = partition::check(partition)?;
        let img = sparse::parse(src, 4096)?;
        let pieces = sparse::plan(&img, self.max_download()?)?;
        let n = pieces.len();
        for (i, p) in pieces.iter().enumerate() {
            let total = p.bytes;
            self.download(total, &mut |sink| sparse::write_piece(&img, p, src, sink), &mut |d| progress(Progress::Sent { piece: i + 1, pieces: n, done: d, total }))?;
            progress(Progress::Flashing { piece: i + 1, pieces: n });
            self.run(&format!("flash:{part}"), Duration::from_secs(600), &mut |p| progress(p))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn allowlist() {
        for ok in ["getvar:max-download-size", "getvar:partition-size:userdata", "download:0000ffff", "flash:userdata", "flash:boot", "flash:logo", "boot", "reboot", "upload", "oem hash:userdata:0:40000000:10000"] {
            assert!(command(ok).is_ok(), "{ok}");
        }
        for bad in ["flash:modem", "flash:persist", "flash:abl", "erase:userdata", "format:userdata", "oem unlock", "flashing unlock", "set_active:a", "oem hash:modem:0:1:1", "download:ffff", "getvar:a b", "flash:userdata;reboot", "oem hash:boot:0:1"] {
            assert!(command(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn replies() {
        assert_eq!(parse_reply(b"OKAY0.4").unwrap(), Reply::Okay("0.4".into()));
        assert_eq!(parse_reply(b"DATA00001000").unwrap(), Reply::Data(0x1000));
        assert_eq!(parse_reply(b"INFOhello").unwrap(), Reply::Info("hello".into()));
        assert!(parse_reply(b"WHAT").is_err());
    }

    /// A fake fastboot device: records what it receives, answers like Enxerto, applies flashes.
    struct Dev {
        out: VecDeque<Vec<u8>>,
        expect: u64,
        got: Vec<u8>,
        part: Vec<Option<Vec<u8>>>,
        cmds: Vec<String>,
    }
    impl Transport for Dev {
        fn send(&mut self, b: &[u8]) -> std::io::Result<()> {
            if self.expect > 0 {
                self.got.extend_from_slice(b);
                self.expect -= b.len() as u64;
                if self.expect == 0 {
                    self.out.push_back(b"OKAY".to_vec());
                }
                return Ok(());
            }
            let c = String::from_utf8(b.to_vec()).unwrap();
            self.cmds.push(c.clone());
            let r = if c == "getvar:max-download-size" {
                "OKAY0x400".to_string()
            } else if let Some(n) = c.strip_prefix("download:") {
                self.expect = u64::from_str_radix(n, 16).unwrap();
                self.got.clear();
                format!("DATA{n}")
            } else if c.starts_with("flash:") {
                crate::sparse::apply(&mut self.part, &self.got).unwrap();
                "OKAY".into()
            } else {
                "OKAY".into()
            };
            self.out.push_back(r.into_bytes());
            Ok(())
        }
        fn recv(&mut self, _: Duration) -> std::io::Result<Option<Vec<u8>>> {
            Ok(self.out.pop_front())
        }
    }

    #[test]
    fn flash_in_pieces() {
        let blocks: Vec<Option<Vec<u8>>> = (0..30u8).map(|i| if i % 5 == 0 { None } else { Some((0..64u8).map(|b| b.wrapping_mul(i)).collect()) }).collect();
        let img = crate::sparse::build(64, &blocks);
        let mut fb = Fastboot::new(Dev { out: VecDeque::new(), expect: 0, got: vec![], part: vec![None; 30], cmds: vec![] });
        let mut src: &[u8] = &img;
        fb.flash("userdata", &mut src, &mut |_| {}).unwrap();
        assert_eq!(fb.t.part, blocks);
        assert!(fb.t.cmds.iter().filter(|c| c.as_str() == "flash:userdata").count() > 1);
        let mut src: &[u8] = &img;
        assert!(matches!(fb.flash("modem", &mut src, &mut |_| {}), Err(Error::Refused(_))));
        assert!(!fb.t.cmds.iter().any(|c| c.contains("modem")));
    }
}
