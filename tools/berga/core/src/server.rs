//! Phone side of the daemon ops (op >= 0x100): one request frame in, one response or error frame out.
//! The phone daemon (`bergad`, tools/berga/phone) and the fake phone both answer through [`handle`], so the
//! allowlists below are enforced on the phone whatever the host sends. The host checks the same lists first
//! (nicer errors, nothing sent).

use crate::proto::{Frame, Kind, op, CHUNK};
use serde_json::{Value, json};

/// Phone tools RUN may start: `/usr/local/bin/<name>` (Bergamota's own tools) or the system path in [`run_path`].
pub const RUN_ALLOWED: [&str; 7] = ["bergamotametrics", "bergamotashot", "bergamotauicache", "sysctl", "uptime", "df", "ps"];

/// Absolute path of an allowed command, `None` for everything else.
pub fn run_path(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "bergamotametrics" => "/usr/local/bin/bergamotametrics",
        "bergamotashot" => "/usr/local/bin/bergamotashot",
        "bergamotauicache" => "/usr/local/bin/bergamotauicache",
        "sysctl" => "/usr/sbin/sysctl",
        "uptime" => "/usr/bin/uptime",
        "df" => "/bin/df",
        "ps" => "/bin/ps",
        _ => return None,
    })
}

/// Argument checks per command: sysctl may only read (no `name=value`, no -w).
pub fn run_allowed(cmd: &str, args: &[String]) -> Result<(), String> {
    if run_path(cmd).is_none() {
        return Err(format!("{cmd} is not an allowed command ({})", RUN_ALLOWED.join(", ")));
    }
    if args.len() > 16 || args.iter().any(|a| a.len() > 1024 || a.contains('\0')) {
        return Err("too many or too long arguments".into());
    }
    if cmd == "sysctl" && args.iter().any(|a| a.contains('=') || a.starts_with("-w")) {
        return Err("sysctl may only read".into());
    }
    Ok(())
}

/// Where PUSH may write: media, the app install area, berga's own scratch directory.
pub const PUSH_ROOTS: [&str; 3] = ["/var/mobile/Media/", "/Applications/", "/var/root/berga/"];
/// Where PULL may read: the above plus logs and crash reports.
pub const PULL_ROOTS: [&str; 5] = ["/var/mobile/Media/", "/Applications/", "/var/root/berga/", "/var/log/", "/var/mobile/Library/Logs/CrashReporter/"];

/// Absolute, no `..` / `.` / empty components, under one of `roots` (`/private/var` is accepted for `/var`).
pub fn path_allowed(path: &str, roots: &[&str]) -> Result<String, String> {
    let p = path.strip_prefix("/private").filter(|r| r.starts_with("/var/")).unwrap_or(path);
    if !p.starts_with('/') || p.len() > 1024 || p.contains('\0') {
        return Err(format!("{path}: must be an absolute path"));
    }
    if p[1..].split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err(format!("{path}: no empty, . or .. components"));
    }
    if !roots.iter().any(|r| p.starts_with(r) && p.len() > r.len()) {
        return Err(format!("{path}: outside {}", roots.join(", ")));
    }
    Ok(p.to_string())
}

/// What the phone (or the fake) actually does. Paths given here are already checked.
pub trait Backend {
    fn status(&mut self) -> Result<Value, String>;
    /// `bergamotametrics` JSON (schema of apps/Bergamota Core/Metrics.swift).
    fn metrics(&mut self) -> Result<Value, String>;
    /// Exit code and combined output, at most 15 s.
    fn run(&mut self, path: &str, args: &[String]) -> Result<(i32, Vec<u8>), String>;
    /// Writes `data` at `offset`; `truncate` starts a new file (creating parent directories).
    fn write_file(&mut self, path: &str, offset: u64, truncate: bool, mode: u32, data: &[u8]) -> Result<(), String>;
    /// File size and up to `len` bytes from `offset`.
    fn read_file(&mut self, path: &str, offset: u64, len: usize) -> Result<(u64, Vec<u8>), String>;
    /// PNG of the screen.
    fn screenshot(&mut self) -> Result<Vec<u8>, String>;
    /// Moves a pushed picture/video into the camera roll; returns where it went.
    fn import_media(&mut self, path: &str) -> Result<Value, String>;
}

fn s(head: &Value, k: &str) -> Option<String> {
    head.get(k).and_then(Value::as_str).map(str::to_string)
}
fn u(head: &Value, k: &str) -> u64 {
    head.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Answers one daemon request.
pub fn handle(b: &mut dyn Backend, req: &Frame) -> Frame {
    let ok = |head: Value, body: Vec<u8>| Frame::new(Kind::Response, req.op, req.tag, head, body);
    let r: Result<Frame, String> = (|| {
        if req.kind != Kind::Request {
            return Err("not a request".into());
        }
        let h = &req.head;
        match req.op {
            op::STATUS => Ok(ok(b.status()?, vec![])),
            op::METRICS => Ok(ok(b.metrics()?, vec![])),
            op::RUN => {
                let cmd = s(h, "cmd").ok_or("run needs \"cmd\"")?;
                let args: Vec<String> = h.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                run_allowed(&cmd, &args)?;
                let (code, mut out) = b.run(run_path(&cmd).unwrap(), &args)?;
                let truncated = out.len() > CHUNK;
                out.truncate(CHUNK);
                Ok(ok(json!({ "code": code, "truncated": truncated }), out))
            }
            op::PUSH => {
                let path = path_allowed(&s(h, "path").ok_or("push needs \"path\"")?, &PUSH_ROOTS)?;
                let mode = h.get("mode").and_then(Value::as_u64).unwrap_or(0o644) as u32 & 0o755;
                b.write_file(&path, u(h, "offset"), h.get("truncate").and_then(Value::as_bool).unwrap_or(false), mode, &req.body)?;
                Ok(ok(json!({ "written": req.body.len() }), vec![]))
            }
            op::PULL => {
                let path = path_allowed(&s(h, "path").ok_or("pull needs \"path\"")?, &PULL_ROOTS)?;
                let len = (u(h, "len") as usize).clamp(1, CHUNK);
                let off = u(h, "offset");
                let (size, data) = b.read_file(&path, off, len)?;
                Ok(ok(json!({ "size": size, "eof": off + data.len() as u64 >= size }), data))
            }
            op::SCREENSHOT => {
                let png = b.screenshot()?;
                if png.len() > CHUNK {
                    // shortcut: one frame (60 KiB) per screenshot; a larger PNG would need PULL of a staged file
                    return Err(format!("screenshot is {} bytes (max {CHUNK} per frame)", png.len()));
                }
                Ok(ok(json!({ "type": "image/png" }), png))
            }
            op::MEDIA_IMPORT => {
                let path = path_allowed(&s(h, "path").ok_or("media-import needs \"path\"")?, &["/var/mobile/Media/berga/"])?;
                Ok(ok(b.import_media(&path)?, vec![]))
            }
            other => Err(format!("unknown op 0x{other:04x}")),
        }
    })();
    r.unwrap_or_else(|e| Frame::error(req.op, req.tag, &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert!(path_allowed("/var/mobile/Media/DCIM/a.jpg", &PUSH_ROOTS).is_ok());
        assert_eq!(path_allowed("/private/var/mobile/Media/x", &PUSH_ROOTS).unwrap(), "/var/mobile/Media/x");
        for bad in ["/var/mobile/Media/../../etc/passwd", "var/mobile/Media/x", "/etc/master.passwd", "/var/mobile/Media/", "/var/mobile/Media//x", "/private/etc/x", "/Applications/./x"] {
            assert!(path_allowed(bad, &PUSH_ROOTS).is_err(), "{bad}");
        }
        assert!(path_allowed("/var/log/system.log", &PUSH_ROOTS).is_err());
        assert!(path_allowed("/var/log/system.log", &PULL_ROOTS).is_ok());
    }

    #[test]
    fn run_rules() {
        assert!(run_allowed("bergamotametrics", &[]).is_ok());
        assert!(run_allowed("sysctl", &["kern.bergamota_usb".into()]).is_ok());
        assert!(run_allowed("sysctl", &["kern.bergamota_reboot_mode=fastboot".into()]).is_err());
        assert!(run_allowed("sh", &["-c".into(), "rm -rf /".into()]).is_err());
        assert!(run_allowed("/bin/sh", &[]).is_err());
    }
}
