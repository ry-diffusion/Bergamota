//! Human-readable views of the phone metrics (`bergamotametrics` JSON, schema of apps/Bergamota Core/Metrics.swift)
//! and of the app's status.json. Same text as the old `bergamota metrics` / `bergamota status`.

use crate::appfiles::{now_unix, parse_iso};
use serde_json::Value;

fn khz(v: Option<f64>) -> String {
    match v {
        None => "unknown".into(),
        Some(v) if v >= 1e6 => format!("{:.2} GHz", v / 1e6),
        Some(v) => format!("{:.0} MHz", v / 1e3),
    }
}
fn pct(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "--".into())
}
fn ago(s: Option<&str>) -> String {
    s.and_then(parse_iso).map(|t| format!("{:.0}s ago", now_unix() - t)).unwrap_or_else(|| "?".into())
}
fn st(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "?".into(),
        other => other.to_string(),
    }
}

pub fn format_metrics(m: &Value) -> String {
    let mut o = Vec::new();
    o.push(format!("sample    {} ({}), schema {}", st(&m["fetched"]), ago(m["fetched"].as_str()), st(&m["schema"])));
    for c in m["cpu"]["clusters"].as_array().into_iter().flatten() {
        let mode = c["mode"].as_str().unwrap_or("unknown");
        let extra = match mode {
            "static" => c["capPercent"].as_f64().map(|p| format!("capped at {p}%")).unwrap_or("fixed".into()),
            "governor" => format!("governor {}", st(&c["governor"])),
            "unknown" => "unknown (fixed by bootloader)".into(),
            m => m.into(),
        };
        let rng = if c["minKhz"].is_number() || c["maxKhz"].is_number() { format!(" min {} max {}", khz(c["minKhz"].as_f64()), khz(c["maxKhz"].as_f64())) } else { String::new() };
        let cpus = c["cpus"].as_array();
        let first_last = cpus.map(|a| format!("cpu{}-{}", st(&a[0]), st(&a[a.len() - 1]))).unwrap_or("cpu?-?".into());
        o.push(format!("cpu       {:4} {first_last}  {:>9}  load {:>4}  {extra}{rng}", st(&c["name"]), khz(c["curKhz"].as_f64()), pct(c["loadPct"].as_f64())));
    }
    if let Some(l) = m["cpu"]["loadPct"].as_array().filter(|a| !a.is_empty()) {
        o.push(format!("          per-CPU load {}", l.iter().map(|x| format!("{:.0}", x.as_f64().unwrap_or(0.0))).collect::<Vec<_>>().join(" ")));
    }
    let g = &m["gpu"];
    let lvl = if g["level"].is_null() { String::new() } else { format!("level {}/{}  ", st(&g["level"]), st(&g["levels"])) };
    let rate = if g["mhz"].is_null() && g["busyPct"].is_null() {
        "level/MHz/busy not reported (waiting on kern.bergamota_gpu)".into()
    } else {
        format!("{}  busy {}", khz(g["mhz"].as_f64().map(|v| v * 1000.0)), pct(g["busyPct"].as_f64()))
    };
    o.push(format!("gpu       {:8}  {lvl}{rate}  clients {}", g["state"].as_str().unwrap_or("?"), g["clients"].as_u64().unwrap_or(0)));
    let r = &m["renderer"];
    if !r.is_null() {
        o.push(format!("renderer  {}  (requested {} by {}, injected {}, backboardd GPU client {})", st(&r["label"]), st(&r["requested"]), st(&r["requestedBy"]), st(&r["injected"]), if r["gpuClientOpen"].as_bool() == Some(true) { "yes" } else { "no" }));
    }
    let b = &m["battery"];
    if b["present"].as_bool() == Some(true) {
        let ma = b["currentMa"].as_f64().map(|v| format!("{v:+.0} mA")).unwrap_or("mA unknown".into());
        let v = b["voltageMv"].as_f64().map(|v| format!("{:.2} V", v / 1000.0)).unwrap_or_default();
        o.push(format!("battery   {}  {}  {ma}  {v}", pct(b["percent"].as_f64()), if b["charging"].as_bool() == Some(true) { "charging" } else { "not charging" }));
    } else {
        o.push("battery   not present".into());
    }
    if let Some(t) = m["thermal"]["batteryC"].as_f64() {
        o.push(format!("thermal   battery {t:.1} C"));
    }
    for e in m["errors"].as_array().into_iter().flatten() {
        o.push(format!("note      {}", st(e)));
    }
    o.join("\n")
}

/// status.json of the Mac app.
pub fn format_app_status(d: &Value) -> String {
    let mut o = Vec::new();
    let (p, q, l) = (&d["phone"], &d["queue"], &d["liveLog"]);
    o.push(format!("updated   {} ({})", st(&d["updated"]), ago(d["updated"].as_str())));
    let fb = p["fastbootKind"].as_str().map(|k| format!("  [{} fastboot {}]", k.to_uppercase(), p["fastbootVersion"].as_str().unwrap_or(""))).unwrap_or_default();
    o.push(format!("phone     {} (since {}){fb}", st(&p["state"]), st(&p["since"])));
    if let Some(ios) = p.get("ios").filter(|v| v.is_object()) {
        o.push(format!("  ios     battery={}% charging={} uptime={}s", st(&ios["battery"]), st(&ios["charging"]), st(&ios["uptime"])));
        o.push(format!("          {}", ios["kernel"].as_str().unwrap_or("")));
    }
    o.push(format!(
        "queue     {}  remote-start={}  orangefox-reboot={}",
        if q["armed"].as_bool() == Some(true) { "ARMED" } else { "stopped" },
        if q["remoteStartAllowed"].as_bool() == Some(true) { "allowed" } else { "denied" },
        st(&q["orangefoxReboot"])
    ));
    o.push(format!("          {}", q["status"].as_str().unwrap_or("")));
    for it in q["items"].as_array().into_iter().flatten() {
        let mut line = format!("  [{:7}] {}  {}", st(&it["state"]), st(&it["label"]), st(&it["image"]));
        if let Some(s) = it["sentAt"].as_str() {
            line += &format!("  sent {s}");
        }
        if it["needsManualFastboot"].as_bool() == Some(true) {
            line += "  NEEDS MANUAL FASTBOOT";
        }
        o.push(line);
        if let Some(f) = it["flashUserdata"].as_str() {
            o.push(format!("             flashes {}: {f}  (via {}, {})", it["flashPartition"].as_str().unwrap_or("userdata"), st(&it["flashVia"]), st(&it["flashMode"])));
        }
        if let (Some("sent" | "failed"), Some(r)) = (it["state"].as_str(), it["result"].as_str()) {
            let lines: Vec<&str> = r.trim().lines().collect();
            for l in &lines[lines.len().saturating_sub(3)..] {
                o.push(format!("             {l}"));
            }
        }
    }
    let fl = &d["flash"];
    if fl["active"].as_bool() == Some(true) {
        o.push(format!("flash     FLASHING {} {}  {}", fl["partition"].as_str().unwrap_or("userdata").to_uppercase(), st(&fl["progress"]), st(&fl["file"])));
    } else if fl["last"].is_object() {
        o.push(format!("flash     {} {}  {}", st(&fl["last"]["result"]), st(&fl["last"]["at"]), st(&fl["last"]["message"])));
    }
    o.push(format!("liveLog   listening={} saving={} {} lines/s", st(&l["listening"]), st(&l["savingTo"]), st(&l["linesPerSecond"])));
    o.push(format!("build     {}  last={}", if d["build"]["building"].as_bool() == Some(true) { "RUNNING" } else { "idle" }, st(&d["build"]["last"])));
    o.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_fake_metrics() {
        let mut b = crate::fake::FakePhone::new();
        let m = crate::server::Backend::metrics(&mut b.backend).unwrap();
        let t = format_metrics(&m);
        assert!(t.contains("cpu       big  cpu4-7"), "{t}");
        assert!(t.contains("battery   81%  charging  +640 mA  4.18 V"), "{t}");
        assert!(format_metrics(&serde_json::json!({})).contains("battery   not present"));
    }
}
