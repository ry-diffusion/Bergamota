//! The boot/flash queue: same file and rules as the Bergamota app (apps/Bergamota Core/BootQueue.swift).
//! `queue.json` = `{"items": [QueueItem], "armed": bool}` in Swift Codable form (every non-optional key present,
//! dates as seconds since 2001-01-01, UUIDs uppercase).
//! Rules: nothing is sent unless the queue is armed; exactly ONE item per fastboot appearance; a failed send is
//! never retried (the phone has to leave fastboot first); an item interrupted mid-send is marked failed on load.

use serde::{Deserialize, Serialize};

/// Seconds between 1970-01-01 and Swift's reference date 2001-01-01.
pub const SWIFT_EPOCH: f64 = 978_307_200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Sending,
    Sent,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    /// Boot image to `fastboot boot`; empty = flash only.
    pub image_path: String,
    pub label: String,
    pub size: i64,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sent_at: Option<f64>,
    pub output: String,
    pub auto_reboot_tried: bool,
    pub needs_manual_fastboot: bool,
    /// Image flashed before the boot (boot/userdata/logo only, see `flash_partition`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub userdata_image: Option<String>,
    pub userdata_size: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub flash_partition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub flash_via: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub flash_mode: Option<String>,
}

impl QueueItem {
    pub fn boot(id: String, image: &str, size: i64, label: &str) -> QueueItem {
        QueueItem {
            id,
            image_path: image.into(),
            label: label.into(),
            size,
            status: Status::Pending,
            sent_at: None,
            output: String::new(),
            auto_reboot_tried: false,
            needs_manual_fastboot: false,
            userdata_image: None,
            userdata_size: 0,
            flash_partition: None,
            flash_via: None,
            flash_mode: None,
        }
    }
    pub fn flash_target(&self) -> &str {
        self.flash_partition.as_deref().unwrap_or("userdata")
    }
    pub fn name(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
        let mut parts = Vec::new();
        if let Some(u) = &self.userdata_image {
            parts.push(format!("flash {} {}", self.flash_target(), base(u)));
        }
        if !self.image_path.is_empty() {
            parts.push(format!("{}{}", if self.userdata_image.is_some() { "then boot " } else { "" }, base(&self.image_path)));
        }
        parts.join(", ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    pub items: Vec<QueueItem>,
    pub armed: bool,
}

/// What the runner sees of the phone (from USB enumeration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Absent,
    Ios,
    Fastboot { bergamota: bool },
}

impl Queue {
    pub fn parse(json: &str) -> serde_json::Result<Queue> {
        let mut q: Queue = serde_json::from_str(json)?;
        for it in q.items.iter_mut().filter(|i| i.status == Status::Sending) {
            it.status = Status::Failed;
            it.output.push_str("\nberga: interrupted during the send (not resent)");
        }
        Ok(q)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("queue serializes")
    }
    pub fn pending(&self) -> impl Iterator<Item = (usize, &QueueItem)> {
        self.items.iter().enumerate().filter(|(_, i)| i.status == Status::Pending)
    }
    pub fn finish(&mut self, idx: usize, ok: bool, output: &str, now_unix: f64) {
        let it = &mut self.items[idx];
        it.status = if ok { Status::Sent } else { Status::Failed };
        it.sent_at = Some(now_unix - SWIFT_EPOCH);
        it.output.push_str(output);
    }
}

/// Decides when to send. Keep one per process watching the phone.
#[derive(Default)]
pub struct Runner {
    /// An item went out in this fastboot session: nothing more until the phone leaves fastboot.
    waiting_for_leave: bool,
}

impl Runner {
    /// Index of the item to send now (already marked `Sending`), or `None`.
    pub fn step(&mut self, q: &mut Queue, seen: Seen) -> Option<usize> {
        match seen {
            Seen::Fastboot { .. } => {
                if !q.armed || self.waiting_for_leave {
                    return None;
                }
                let idx = q.pending().next()?.0;
                self.waiting_for_leave = true;
                q.items[idx].status = Status::Sending;
                Some(idx)
            }
            _ => {
                self.waiting_for_leave = false;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swift_shape_roundtrip() {
        let swift = r#"{"armed":false,"items":[{"output":"","needsManualFastboot":false,"userdataSize":0,"status":"sending","label":"x","imagePath":"/a/boot.img","autoRebootTried":false,"id":"56C3516A-6DFF-4B11-9C0C-265E2071DE12","sentAt":814224281.007853,"size":51187712}]}"#;
        let q = Queue::parse(swift).unwrap();
        assert_eq!(q.items[0].status, Status::Failed);
        let back: serde_json::Value = serde_json::from_str(&q.to_json()).unwrap();
        let it = &back["items"][0];
        for k in ["id", "imagePath", "label", "size", "status", "output", "autoRebootTried", "needsManualFastboot", "userdataSize"] {
            assert!(it.get(k).is_some(), "{k} must always be written (Swift decode needs it)");
        }
        assert!(it.get("flashPartition").is_none());
    }

    #[test]
    fn one_send_per_fastboot_appearance() {
        let mut q = Queue { items: vec![QueueItem::boot("A".into(), "/a", 1, ""), QueueItem::boot("B".into(), "/b", 1, "")], armed: false };
        let mut r = Runner::default();
        let fb = Seen::Fastboot { bergamota: false };
        assert_eq!(r.step(&mut q, fb), None, "disarmed");
        q.armed = true;
        assert_eq!(r.step(&mut q, fb), Some(0));
        q.finish(0, false, "boom", 1e9);
        assert_eq!(r.step(&mut q, fb), None, "same fastboot session");
        assert_eq!(r.step(&mut q, Seen::Absent), None);
        assert_eq!(r.step(&mut q, fb), Some(1), "failed item is not retried, next one goes");
        q.finish(1, true, "ok", 1e9);
        assert_eq!(q.items[0].status, Status::Failed);
        assert_eq!(q.items[1].status, Status::Sent);
        assert_eq!(r.step(&mut q, Seen::Ios), None);
        assert_eq!(r.step(&mut q, fb), None, "nothing pending");
    }
}
