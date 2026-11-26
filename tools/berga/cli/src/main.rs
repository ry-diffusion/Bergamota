//! berga: the Bergamota command line (replaces apps/Bergamota/scripts/bergamota). See tools/berga/README.md.

use berga_core::appfiles::{self, app_running};
use berga_core::client::Incoming;
use berga_core::fastboot::{Fastboot, Progress};
use berga_core::queue::{Queue, QueueItem, Runner, Seen};
use berga_core::session::Session;
use berga_core::transport::Transport;
use berga_core::usb::{self, UsbPipe};
use berga_core::{Error, bootimg, fake::FakeTransport, metrics, partition, presets};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "\
berga: manage the Bergamota phone (Poco F1 running iOS) over USB

Phone (berga USB interface of the iOS kernel, patch 1037; --fake uses a built-in fake phone):
  berga devices                         berga / fastboot interfaces on USB
  berga hello                           kernel version, uptime, daemon running
  berga status                          phone status (berga USB), else the Mac app's status.json
  berga metrics [--json]                CPU/GPU/battery (berga USB), else the app's last sample
  berga log [--backlog BYTES]           live kernel log until Ctrl-C
  berga run CMD [ARGS..]                allowed phone tool (bergamotametrics, sysctl, uptime, df, ps, ...)
  berga screenshot [-o FILE.png]
  berga push LOCAL REMOTE | pull REMOTE [LOCAL]
  berga upload [--app] FILE..           pictures/videos into the camera roll
  berga reboot normal|fastboot|bergamota|recovery|poweroff [--app]

Queue (boots and flashes go only through the queue; the Mac app runs it, or `berga queue run`):
  berga queue [--json]                  items and armed state
  berga enqueue BOOT.img [--label L] [--no-arm] [--wait]
  berga boot BOOT.img (--preset NAME | --cmdline \"...\") [--label L] [--no-arm] [--wait]
                                        writes BOOT-<label>.img with new xnu boot-args, enqueues it
  berga flash IMG [--partition userdata|boot|logo] [--then-boot BOOT.img] [--full|--delta]
              [--via abl|bergamota|auto] [--label L] [--no-arm] [--wait]
  berga queue arm|disarm                (only while the app is not running)
  berga queue run                       send queued items here (app not running): one per fastboot appearance
  berga install APP.ipa [--wait]        via the Mac app (pacstrip + re-sign + register)

Images and settings:
  berga bootimg BOOT.img                Android header + xnu boot-args of a boot image
  berga presets                         cmdline presets (presets.json)

Files: $BERGAMOTA_HOME or ~/Library/Application Support/Bergamota (shared with the Mac app).";

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("berga: {msg}");
    std::process::exit(1)
}

struct Args {
    v: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        let n = self.v.len();
        self.v.retain(|a| a != name);
        n != self.v.len()
    }
    fn opt(&mut self, name: &str) -> Option<String> {
        let i = self.v.iter().position(|a| a == name)?;
        if i + 1 >= self.v.len() {
            die(format!("{name} needs a value"));
        }
        let val = self.v.remove(i + 1);
        self.v.remove(i);
        Some(val)
    }
    fn rest(&mut self) -> Vec<String> {
        if let Some(a) = self.v.iter().find(|a| a.starts_with("--") && a.len() > 2) {
            die(format!("unknown option {a}"));
        }
        std::mem::take(&mut self.v)
    }
}

fn session(fake: bool) -> Session<Box<dyn Transport>> {
    let t: Box<dyn Transport> = if fake { Box::new(FakeTransport::default()) } else { Box::new(UsbPipe::berga().unwrap_or_else(|e| die(e))) };
    Session::new(t)
}

/// A berga session when a berga phone (or --fake) is there, else `None` (callers fall back to the app).
fn try_session(fake: bool) -> Option<Session<Box<dyn Transport>>> {
    if fake {
        return Some(session(true));
    }
    let found = usb::list().ok()?.into_iter().find(|f| f.kind == usb::Kind::Berga)?;
    UsbPipe::open(&found, berga_core::proto::MAX_FRAME).ok().map(|p| Session::new(Box::new(p) as Box<dyn Transport>))
}

fn base() -> PathBuf {
    appfiles::base_dir()
}

fn app_status() -> Option<Value> {
    appfiles::status(&base()).ok()
}

fn abs(p: &str) -> String {
    std::fs::canonicalize(p).map(|p| p.display().to_string()).unwrap_or_else(|_| die(format!("{p}: no such file")))
}

fn progress_bar(what: &str) -> impl FnMut(u64, u64) {
    let what = what.to_string();
    move |d, t| {
        eprint!("\r{what}: {} / {} KiB", d >> 10, t >> 10);
        if d >= t {
            eprintln!();
        }
    }
}

/// Request to the Mac app's inbox, optionally waiting for its result.
fn submit_app(req: Value, wait: bool) {
    let b = base();
    let id = appfiles::submit(&b, &req).unwrap_or_else(|e| die(format!("inbox: {e}")));
    println!("submitted {id}");
    if wait {
        match appfiles::wait(&b, &id, Duration::from_secs(1800)) {
            Ok(r) => println!("OK: {r}"),
            Err(e) => die(format!("ERROR: {e}")),
        }
    }
}

fn load_queue() -> Queue {
    match std::fs::read_to_string(base().join("queue.json")) {
        Ok(s) => Queue::parse(&s).unwrap_or_else(|e| die(format!("queue.json: {e}"))),
        Err(_) => Queue::default(),
    }
}

fn save_queue(q: &Queue) {
    let p = base().join("queue.json");
    let tmp = p.with_extension("json.berga-tmp");
    std::fs::create_dir_all(base()).and_then(|_| std::fs::write(&tmp, q.to_json())).and_then(|_| std::fs::rename(&tmp, &p)).unwrap_or_else(|e| die(format!("queue.json: {e}")));
}

fn uuid() -> String {
    let mut b = [0u8; 16];
    // shortcut: /dev/urandom (macOS/Linux only); the CLI is native-only
    std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).unwrap_or_else(|e| die(e));
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02X}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// The app takes queue changes through its inbox while it runs; otherwise berga edits queue.json itself.
fn enqueue(req: Value, item: QueueItem, wait: bool, arm: bool) {
    if app_status().is_some_and(|s| app_running(&s)) {
        return submit_app(req, wait);
    }
    let mut q = load_queue();
    println!("queued {} ({}; the Mac app is not running: `berga queue run` sends it)", item.name(), item.id);
    q.items.push(item);
    q.armed |= arm;
    save_queue(&q);
}

fn file_size(p: &str) -> i64 {
    std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or_else(|e| die(format!("{p}: {e}")))
}

fn main() {
    let mut a = Args { v: std::env::args().skip(1).collect() };
    let fake = a.flag("--fake");
    if a.v.is_empty() || a.flag("-h") || a.flag("--help") {
        println!("{USAGE}");
        return;
    }
    let cmd = a.v.remove(0);
    match cmd.as_str() {
        "help" => println!("{USAGE}"),
        "devices" => {
            let l = usb::list().unwrap_or_else(|e| die(e));
            if l.is_empty() {
                println!("no berga or fastboot interface on USB");
            }
            for f in l {
                println!("{:?}\t{:04x}:{:04x}\tif {}\t{}\t{}", f.kind, f.vid, f.pid, f.interface, f.product.unwrap_or_default(), f.serial.unwrap_or_default());
            }
        }
        "hello" => println!("{}", serde_json::to_string_pretty(&session(fake).hello().unwrap_or_else(|e| die(e))).unwrap()),
        "status" => match try_session(fake) {
            Some(mut s) => {
                let h = s.hello().unwrap_or_else(|e| die(e));
                println!("kernel    {}", h["kernel"].as_str().unwrap_or("?"));
                println!("uptime    {} s   daemon {}", h["uptime_ms"].as_u64().unwrap_or(0) / 1000, h["daemon"]);
                match s.status() {
                    Ok(st) => println!("{}", serde_json::to_string_pretty(&st).unwrap()),
                    Err(e) => println!("status    {e}"),
                }
            }
            None => match app_status() {
                Some(d) => println!("{}\n(from the Mac app's status.json; no berga phone on USB)", metrics::format_app_status(&d)),
                None => die("no berga phone on USB and no status.json from the Mac app"),
            },
        },
        "metrics" => {
            let json_out = a.flag("--json");
            let m = match try_session(fake) {
                Some(mut s) => s.metrics().unwrap_or_else(|e| die(e)),
                None => {
                    let d = app_status().unwrap_or_else(|| die("no berga phone on USB and no status.json from the Mac app"));
                    let m = d["phone"]["metrics"].clone();
                    if m.is_null() {
                        die(format!("no metrics: {}", d["phone"]["metricsError"].as_str().unwrap_or("phone state is not iOS")));
                    }
                    m
                }
            };
            if json_out { println!("{}", serde_json::to_string_pretty(&m).unwrap()) } else { println!("{}", metrics::format_metrics(&m)) }
        }
        "log" => {
            let backlog = a.opt("--backlog").map(|b| b.parse().unwrap_or_else(|_| die("--backlog BYTES"))).unwrap_or(16384);
            let mut s = session(fake);
            s.log_start(backlog).unwrap_or_else(|e| die(e));
            let out = std::io::stdout();
            loop {
                match s.next_event(Duration::from_secs(1)) {
                    Ok(Some(Incoming::Event { body, .. })) => {
                        let mut o = out.lock();
                        let _ = o.write_all(&body);
                        let _ = o.flush();
                    }
                    Ok(_) => {}
                    Err(e) => die(e),
                }
            }
        }
        "run" => {
            let mut r = a.rest();
            if r.is_empty() {
                die("run CMD [ARGS..]");
            }
            let c = r.remove(0);
            let (code, out) = session(fake).run(&c, &r).unwrap_or_else(|e| die(e));
            std::io::stdout().write_all(&out).ok();
            std::process::exit(code);
        }
        "screenshot" => {
            let out = a.opt("-o").unwrap_or_else(|| "berga-screenshot.png".into());
            let png = session(fake).screenshot().unwrap_or_else(|e| die(e));
            std::fs::write(&out, &png).unwrap_or_else(|e| die(e));
            println!("{out} ({} bytes)", png.len());
        }
        "push" => {
            let r = a.rest();
            let [local, remote] = r.as_slice() else { die("push LOCAL REMOTE") };
            let data = std::fs::read(local).unwrap_or_else(|e| die(format!("{local}: {e}")));
            session(fake).push(remote, &data, 0o644, &mut progress_bar("push")).unwrap_or_else(|e| die(e));
        }
        "pull" => {
            let r = a.rest();
            let (remote, local) = match r.as_slice() {
                [r] => (r.clone(), r.rsplit('/').next().unwrap_or("pulled").to_string()),
                [r, l] => (r.clone(), l.clone()),
                _ => die("pull REMOTE [LOCAL]"),
            };
            let data = session(fake).pull(&remote, &mut progress_bar("pull")).unwrap_or_else(|e| die(e));
            std::fs::write(&local, data).unwrap_or_else(|e| die(e));
        }
        "upload" => {
            let via_app = a.flag("--app");
            let dest = a.opt("--dest").unwrap_or_else(|| "photos".into());
            let wait = a.flag("--wait");
            let files = a.rest();
            if files.is_empty() {
                die("upload FILE..");
            }
            if via_app || dest != "photos" {
                return submit_app(json!({ "kind": "upload", "dest": dest, "paths": files.iter().map(|f| abs(f)).collect::<Vec<_>>() }), wait);
            }
            let mut s = session(fake);
            for f in files {
                let data = std::fs::read(&f).unwrap_or_else(|e| die(format!("{f}: {e}")));
                let name = Path::new(&f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let r = s.upload_media(&name, &data, &mut progress_bar(&name)).unwrap_or_else(|e| die(e));
                println!("{f} -> {}", r["path"].as_str().unwrap_or("?"));
            }
        }
        "reboot" => {
            let via_app = a.flag("--app");
            let wait = a.flag("--wait");
            let r = a.rest();
            let [mode] = r.as_slice() else { die("reboot normal|fastboot|bergamota|recovery|poweroff") };
            if via_app {
                if !["fastboot", "bergamota", "recovery"].contains(&mode.as_str()) {
                    die("the app reboots to fastboot, bergamota or recovery");
                }
                return submit_app(json!({ "kind": "reboot", "to": mode }), wait);
            }
            session(fake).reboot(mode).unwrap_or_else(|e| die(e));
            println!("rebooting to {mode}");
        }
        "queue" => {
            let sub = a.v.first().cloned();
            match sub.as_deref() {
                Some("run") => queue_run(),
                Some(s @ ("arm" | "disarm")) => {
                    if app_status().is_some_and(|s| app_running(&s)) {
                        die("the Mac app is running: arm/stop the queue there");
                    }
                    let mut q = load_queue();
                    q.armed = s == "arm";
                    save_queue(&q);
                    println!("queue {}", if q.armed { "ARMED" } else { "disarmed" });
                }
                _ => {
                    let q = load_queue();
                    if a.flag("--json") {
                        println!("{}", q.to_json());
                        return;
                    }
                    println!("queue {}", if q.armed { "ARMED" } else { "stopped" });
                    for it in &q.items {
                        println!("  [{:7}] {}  {}", format!("{:?}", it.status).to_lowercase(), it.name(), it.image_path);
                        if let Some(u) = &it.userdata_image {
                            println!("             flashes {}: {u}", it.flash_target());
                        }
                    }
                }
            }
        }
        "enqueue" => {
            let label = a.opt("--label").unwrap_or_default();
            let (arm, wait) = (!a.flag("--no-arm"), a.flag("--wait"));
            let r = a.rest();
            let [img] = r.as_slice() else { die("enqueue BOOT.img") };
            let img = abs(img);
            let item = QueueItem::boot(uuid(), &img, file_size(&img), &label);
            enqueue(json!({ "kind": "enqueue", "image": img, "label": label, "arm": arm }), item, wait, arm);
        }
        "boot" => {
            let label = a.opt("--label").unwrap_or_default();
            let cmdline = match (a.opt("--cmdline"), a.opt("--preset")) {
                (Some(c), None) => c,
                (None, Some(p)) => {
                    let all = presets::load(&base()).unwrap_or_else(|e| die(format!("presets.json: {e}")));
                    presets::find(&all, &p).unwrap_or_else(|| die(format!("no (unique) preset {p:?}"))).cmdline.clone()
                }
                _ => die("boot needs --preset NAME or --cmdline \"...\""),
            };
            let (arm, wait) = (!a.flag("--no-arm"), a.flag("--wait"));
            let r = a.rest();
            let [img] = r.as_slice() else { die("boot BOOT.img --preset|--cmdline") };
            let data = std::fs::read(img).unwrap_or_else(|e| die(format!("{img}: {e}")));
            let new = bootimg::with_xnu_cmdline(&data, &cmdline).unwrap_or_else(|e| die(e));
            let tag: String = if label.is_empty() { "cmdline".into() } else { label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect() };
            let out = abs(img).trim_end_matches(".img").to_string() + &format!("-{tag}.img");
            std::fs::write(&out, &new).unwrap_or_else(|e| die(e));
            println!("wrote {out}");
            let item = QueueItem::boot(uuid(), &out, new.len() as i64, &label);
            enqueue(json!({ "kind": "enqueue", "image": out, "label": label, "arm": arm }), item, wait, arm);
        }
        "flash" | "flash-userdata" => {
            let part = a.opt("--partition").unwrap_or_else(|| "userdata".into());
            partition::check(&part).unwrap_or_else(|e| die(e));
            let then = a.opt("--then-boot").map(|b| abs(&b));
            let via = a.opt("--via");
            if via.as_deref().is_some_and(|v| !["abl", "bergamota", "auto"].contains(&v)) {
                die("--via must be abl, bergamota or auto");
            }
            let mode = if a.flag("--full") { Some("full") } else if a.flag("--delta") { Some("delta") } else { None };
            let label = a.opt("--label").unwrap_or_default();
            let (arm, wait) = (!a.flag("--no-arm"), a.flag("--wait"));
            let r = a.rest();
            let [img] = r.as_slice() else { die("flash IMG [--partition P]") };
            let img = abs(img);
            let mut head = [0u8; 8];
            std::fs::File::open(&img).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or_else(|e| die(e));
            partition::validate_image(&part, file_size(&img) as u64, &head).unwrap_or_else(|e| die(e));
            // The CLI invocation is the explicit act: it adds "confirm" itself, like the old script.
            let mut req = json!({ "kind": "flash_userdata", "path": img, "partition": part, "confirm": part, "arm": arm });
            for (k, v) in [("then_boot", then.clone()), ("label", Some(label.clone()).filter(|l| !l.is_empty())), ("mode", mode.map(str::to_string)), ("via", via.clone())] {
                if let Some(v) = v {
                    req[k] = json!(v);
                }
            }
            let mut item = QueueItem::boot(uuid(), then.as_deref().unwrap_or(""), then.as_deref().map(file_size).unwrap_or(0), &label);
            item.userdata_size = file_size(&img);
            item.userdata_image = Some(img);
            item.flash_partition = (part != "userdata").then_some(part);
            item.flash_via = via;
            item.flash_mode = mode.map(str::to_string);
            enqueue(req, item, wait, arm);
        }
        "install" => {
            let wait = a.flag("--wait");
            let r = a.rest();
            let [ipa] = r.as_slice() else { die("install APP.ipa") };
            submit_app(json!({ "kind": "install_ipa", "path": abs(ipa) }), wait);
        }
        "bootimg" => {
            let r = a.rest();
            let [img] = r.as_slice() else { die("bootimg BOOT.img") };
            let b = bootimg::parse(&std::fs::read(img).unwrap_or_else(|e| die(e))).unwrap_or_else(|e| die(e));
            println!("header v{}  page {}  kernel {} B  ramdisk {} B at 0x{:x}", b.header_version, b.page_size, b.kernel_size, b.ramdisk_size, b.ramdisk_offset);
            println!("android cmdline  {}", b.header_cmdline);
            match b.xnu_cmdline {
                Some(c) => println!("xnu boot-args    {c}  (payload v{})", b.payload_version.unwrap_or(0)),
                None => println!("xnu boot-args    (no Bergamota payload: Bergamota fastboot image?)"),
            }
        }
        "presets" => {
            for p in presets::load(&base()).unwrap_or_else(|e| die(format!("presets.json: {e}"))) {
                println!("{}\n    {}", p.name, p.cmdline);
            }
        }
        other => die(format!("unknown command {other} (berga --help)")),
    }
}

/// The queue without the Mac app: watch USB, send one pending item per fastboot appearance (boot items and full
/// flashes; delta flashes and the ABL -> Bergamota fastboot dance stay with the app for now).
fn queue_run() {
    if app_status().is_some_and(|s| app_running(&s)) {
        die("the Mac app is running and owns the queue (quit it to run the queue here)");
    }
    let mut runner = Runner::default();
    println!("watching USB for fastboot (Ctrl-C to stop)");
    loop {
        let mut q = load_queue();
        let seen = match usb::list() {
            Ok(l) if l.iter().any(|f| f.kind == usb::Kind::Fastboot) => Seen::Fastboot { bergamota: false },
            Ok(l) if l.iter().any(|f| f.kind == usb::Kind::Berga) => Seen::Ios,
            _ => Seen::Absent,
        };
        if let Some(i) = runner.step(&mut q, seen) {
            save_queue(&q);
            let it = q.items[i].clone();
            println!("sending {}", it.name());
            let r = send_item(&it);
            let mut q2 = load_queue();
            if let Some(j) = q2.items.iter().position(|x| x.id == it.id) {
                let (ok, out) = match &r {
                    Ok(s) => (true, s.clone()),
                    Err(e) => (false, format!("FAILED: {e}\n")),
                };
                print!("{out}");
                q2.finish(j, ok, &out, appfiles::now_unix());
                save_queue(&q2);
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn send_item(it: &QueueItem) -> Result<String, Error> {
    let mut fb = Fastboot::new(UsbPipe::fastboot()?);
    let mut log = String::new();
    let mut show = |p: Progress| match p {
        Progress::Sent { piece, pieces, done, total } => eprint!("\r  piece {piece}/{pieces}: {} / {} KiB   ", done >> 10, total >> 10),
        Progress::Flashing { piece, pieces } => eprintln!("\n  writing piece {piece}/{pieces}"),
        Progress::Info(s) => eprintln!("  {s}"),
    };
    if let Some(img) = &it.userdata_image {
        if it.flash_mode.as_deref() == Some("delta") {
            return Err(Error::Refused("delta flashes run in the Mac app (tools/bergamota-flash.py); use --full here".into()));
        }
        let part = partition::check(it.flash_target())?;
        let mut f = std::fs::File::open(img)?;
        fb.flash(part, &mut f, &mut show)?;
        log += &format!("flashed {part} with {img}\n");
    }
    if !it.image_path.is_empty() {
        let data = std::fs::read(&it.image_path)?;
        fb.boot(&data, &mut show)?;
        log += &format!("booted {}\n", it.image_path);
    }
    Ok(log)
}
