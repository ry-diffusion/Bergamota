//! The Bergamota app's file API in `~/Library/Application Support/Bergamota/` (apps/Bergamota/scripts/bergamota):
//! requests are JSON files dropped into `inbox/` (written as `.ID.tmp`, renamed to `ID.json`), results appear in
//! `inbox/done/ID.json` or `inbox/failed/ID.json`; the app publishes `status.json` and keeps `queue.json`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `$BERGAMOTA_HOME`, else the app's folder (macOS) or `$XDG_DATA_HOME/bergamota`.
pub fn base_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("BERGAMOTA_HOME") {
        return PathBuf::from(p);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Bergamota")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("bergamota")
    }
}

pub fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Seconds since 1970 of an ISO 8601 UTC time like `2026-10-28T23:38:17Z` (fraction optional).
pub fn parse_iso(s: &str) -> Option<f64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let d: Vec<i64> = date.split('-').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let (hms, frac) = time.split_once('.').map(|(a, b)| (a, format!("0.{b}"))).unwrap_or((time, "0".into()));
    let t: Vec<i64> = hms.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    // days from civil (Howard Hinnant)
    let (y, m, day) = (d[0] - (d[1] <= 2) as i64, d[1], d[2]);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some((days * 86400 + t[0] * 3600 + t[1] * 60 + t[2]) as f64 + frac.parse::<f64>().ok()?)
}

pub fn read_json(path: &Path) -> std::io::Result<Value> {
    let s = std::fs::read_to_string(path)?;
    serde_json::from_str(&s).map_err(|e| std::io::Error::other(format!("{}: {e}", path.display())))
}

pub fn status(base: &Path) -> std::io::Result<Value> {
    read_json(&base.join("status.json"))
}

/// The app refreshes status.json every few seconds while it runs.
pub fn app_running(status: &Value) -> bool {
    status["updated"].as_str().and_then(parse_iso).is_some_and(|t| (now_unix() - t).abs() < 30.0)
}

/// Drops a request into the inbox; returns its id (file name).
pub fn submit(base: &Path, req: &Value) -> std::io::Result<String> {
    let inbox = base.join("inbox");
    std::fs::create_dir_all(inbox.join("done"))?;
    std::fs::create_dir_all(inbox.join("failed"))?;
    let id = format!("{}-{}-{}.json", now_unix() as u64, std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() % 32768).unwrap_or(0));
    let tmp = inbox.join(format!(".{id}.tmp"));
    std::fs::write(&tmp, serde_json::to_string(req)? + "\n")?;
    std::fs::rename(&tmp, inbox.join(&id))?;
    Ok(id)
}

/// Waits for the app's result: `Ok(result text)` from done/, `Err(error)` from failed/ or on timeout.
pub fn wait(base: &Path, id: &str, timeout: Duration) -> Result<String, String> {
    let until = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < until {
        for d in ["done", "failed"] {
            let p = base.join("inbox").join(d).join(id);
            if let Ok(v) = read_json(&p) {
                return match v.get("error").and_then(Value::as_str) {
                    Some(e) if !e.is_empty() => Err(e.to_string()),
                    _ => Ok(v.get("result").map(|r| r.as_str().map(str::to_string).unwrap_or_else(|| r.to_string())).unwrap_or_default()),
                };
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("timed out waiting for the app (is Bergamota running?)".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn iso() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_iso("2001-01-01T00:00:00Z"), Some(crate::queue::SWIFT_EPOCH));
        assert_eq!(parse_iso("2026-10-28T23:38:17.5Z"), Some(1_793_230_697.5));
        assert_eq!(parse_iso("2026-10-28 23:38:17"), None);
    }

    #[test]
    fn inbox_roundtrip() {
        let dir = std::env::temp_dir().join(format!("berga-inbox-{}", std::process::id()));
        let id = submit(&dir, &json!({"kind": "stop"})).unwrap();
        assert!(dir.join("inbox").join(&id).exists());
        std::fs::write(dir.join("inbox/failed").join(&id), r#"{"error":"nope"}"#).unwrap();
        assert_eq!(wait(&dir, &id, Duration::from_secs(2)), Err("nope".into()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
