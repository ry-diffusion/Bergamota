//! bergad: the phone side of the berga USB protocol (docs/BERGA_USB.md). The kernel (patch 1037) hands every
//! request it does not answer itself to /dev/berga, one frame per read(); bergad answers through
//! berga_core::server (the same code the fake phone uses, allowlists included) and writes the response frame back.
//! Started by launchd (org.bergamota.bergad.plist), root. Plain arm64: no pacstrip needed.
//!   bergad [--device /dev/berga]

use berga_core::proto::{Decoder, MAX_FRAME};
use berga_core::server::{self, Backend};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Phone;

fn run_timeout(path: &str, args: &[String], timeout: Duration) -> Result<(i32, Vec<u8>), String> {
    let mut child = Command::new(path).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("{path}: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{path}: killed after {} s", timeout.as_secs()));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let mut all = out.stdout;
    all.extend_from_slice(&out.stderr);
    Ok((out.status.code().unwrap_or(-1), all))
}

fn sysctl(name: &str) -> Option<String> {
    run_timeout("/usr/sbin/sysctl", &["-n".into(), name.into()], Duration::from_secs(5)).ok().filter(|r| r.0 == 0).map(|r| String::from_utf8_lossy(&r.1).trim().to_string())
}

fn plist_string(path: &str, key: &str) -> Option<String> {
    // shortcut: SystemVersion.plist is XML on iOS; a binary plist would need a real parser
    let s = std::fs::read_to_string(path).ok()?;
    let at = s.find(&format!("<key>{key}</key>"))?;
    let rest = &s[at..];
    let a = rest.find("<string>")? + 8;
    let b = rest[a..].find("</string>")?;
    Some(rest[a..a + b].to_string())
}

impl Backend for Phone {
    fn status(&mut self) -> Result<Value, String> {
        let sv = "/System/Library/CoreServices/SystemVersion.plist";
        let boot = sysctl("kern.boottime").and_then(|s| s.split("sec = ").nth(1).and_then(|r| r.split(',').next()).and_then(|n| n.trim().parse::<u64>().ok()));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let df = run_timeout("/bin/df", &["-k".into(), "/".into(), "/private/var".into()], Duration::from_secs(5)).ok().map(|r| String::from_utf8_lossy(&r.1).to_string());
        let disks: Vec<Value> = df
            .iter()
            .flat_map(|t| t.lines().skip(1))
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f.len() >= 6).then(|| json!({ "mount": f[f.len() - 1], "total": f[1].parse::<u64>().unwrap_or(0) * 1024, "free": f[3].parse::<u64>().unwrap_or(0) * 1024 }))
            })
            .collect();
        Ok(json!({
            "ios": { "version": plist_string(sv, "ProductVersion"), "build": plist_string(sv, "ProductBuildVersion") },
            "kernel": sysctl("kern.version"),
            "model": sysctl("hw.model"),
            "uptime_s": boot.map(|b| now.saturating_sub(b)),
            "disk": disks,
            "usb": sysctl("kern.bergamota_usb"),
            "berga": sysctl("kern.bergamota_berga"),
            "metrics_tool": std::path::Path::new("/usr/local/bin/bergamotametrics").exists(),
        }))
    }

    fn metrics(&mut self) -> Result<Value, String> {
        let (code, out) = run_timeout("/usr/local/bin/bergamotametrics", &[], Duration::from_secs(15))?;
        if code != 0 {
            return Err(format!("bergamotametrics exit {code}: {}", String::from_utf8_lossy(&out).trim()));
        }
        serde_json::from_slice(&out).map_err(|e| format!("bergamotametrics output: {e}"))
    }

    fn run(&mut self, path: &str, args: &[String]) -> Result<(i32, Vec<u8>), String> {
        run_timeout(path, args, Duration::from_secs(15))
    }

    fn write_file(&mut self, path: &str, offset: u64, truncate: bool, mode: u32, data: &[u8]) -> Result<(), String> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(truncate).mode(mode).open(path).map_err(|e| format!("{path}: {e}"))?;
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if offset > len {
            return Err(format!("{path}: write at {offset} past the end ({len})"));
        }
        f.set_len(offset).and_then(|_| f.seek(SeekFrom::Start(offset))).and_then(|_| f.write_all(data)).map_err(|e| format!("{path}: {e}"))?;
        if truncate {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
        }
        Ok(())
    }

    fn read_file(&mut self, path: &str, offset: u64, len: usize) -> Result<(u64, Vec<u8>), String> {
        let mut f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let size = f.metadata().map_err(|e| e.to_string())?.len();
        f.seek(SeekFrom::Start(offset.min(size))).map_err(|e| e.to_string())?;
        let mut buf = Vec::with_capacity(len);
        f.take(len as u64).read_to_end(&mut buf).map_err(|e| format!("{path}: {e}"))?;
        Ok((size, buf))
    }

    fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        let out = "/private/var/root/berga/shot.png";
        let _ = std::fs::create_dir_all("/private/var/root/berga");
        // scale 3: every third pixel (1080x2246 -> 360x749), small enough for one frame
        let (code, log) = run_timeout("/usr/local/bin/bergamotashot", &[out.into(), "3".into()], Duration::from_secs(15))?;
        if code != 0 {
            return Err(format!("bergamotashot exit {code}: {}", String::from_utf8_lossy(&log).trim()));
        }
        std::fs::read(out).map_err(|e| format!("{out}: {e}"))
    }

    fn import_media(&mut self, path: &str) -> Result<Value, String> {
        use std::os::unix::fs::chown;
        let dir = "/private/var/mobile/Media/DCIM/100APPLE";
        std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
        let last = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok()?.file_name().to_str()?.strip_prefix("IMG_")?.split('.').next()?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        let ext = path.rsplit('.').next().filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("JPG").to_ascii_uppercase();
        let dest = format!("{dir}/IMG_{:04}.{ext}", last + 1);
        std::fs::rename(path, &dest).map_err(|e| format!("{path} -> {dest}: {e}"))?;
        let _ = chown(&dest, Some(501), Some(501));
        // assetsd picks camera-roll files up when it scans the library (may need a Photos relaunch)
        Ok(json!({ "path": dest }))
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dev = "/dev/berga".to_string();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--device" => dev = args.next().unwrap_or_else(|| usage()),
            _ => usage(),
        }
    }
    loop {
        // the kernel allows one opener; retry if the node is missing (berga off) or busy
        match std::fs::OpenOptions::new().read(true).write(true).open(&dev) {
            Ok(f) => serve(f),
            Err(e) => eprintln!("bergad: {dev}: {e}"),
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn usage() -> ! {
    eprintln!("usage: bergad [--device /dev/berga]");
    std::process::exit(2)
}

fn serve(mut f: std::fs::File) {
    eprintln!("bergad: serving");
    let mut phone = Phone;
    let mut buf = vec![0u8; MAX_FRAME];
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("bergad: read: {e}");
                return;
            }
        };
        // one read = one whole frame; a fresh decoder keeps one bad frame from poisoning the next
        let mut d = Decoder::new();
        d.push(&buf[..n]);
        let req = match d.next() {
            Ok(Some(r)) => r,
            other => {
                eprintln!("bergad: bad frame from the kernel: {other:?}");
                continue;
            }
        };
        let resp = server::handle(&mut phone, &req);
        match resp.encode() {
            Ok(bytes) => {
                if let Err(e) = f.write_all(&bytes) {
                    eprintln!("bergad: write: {e}");
                }
            }
            Err(e) => eprintln!("bergad: response too large: {e}"),
        }
    }
}
