//! The berga USB protocol (docs/BERGA_USB.md): frames on the vendor interface's bulk pipes.
//!
//! ```text
//! off size
//! 0   4   magic "BRG1"
//! 4   1   kind     1 request, 2 response, 3 event, 4 error
//! 5   1   flags    bit0 PAD: one 0x00 byte follows the body (so the USB transfer ends in a short packet)
//!                  bit1 MORE: more events follow for this tag (log stream)
//! 6   2   op
//! 8   4   tag      request id, echoed by its response/error/events
//! 12  4   head_len JSON head (UTF-8), may be 0
//! 16  4   body_len raw bytes, may be 0
//! 20      head, body, [pad]
//! ```
//! All integers little endian. A whole frame (pad included) is at most [`MAX_FRAME`] bytes.

use serde_json::Value;

pub const MAGIC: &[u8; 4] = b"BRG1";
pub const HEADER_LEN: usize = 20;
/// Kernel buffer size per direction; header + head + body + pad never exceed it.
pub const MAX_FRAME: usize = 65536;
/// Largest body when the head is small: what file transfers use per frame.
pub const CHUNK: usize = 60 * 1024;
/// USB high-speed bulk max packet: a transfer that is a multiple of it needs a PAD byte.
pub const PACKET: usize = 512;

pub const FLAG_PAD: u8 = 1;
pub const FLAG_MORE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Request = 1,
    Response = 2,
    Event = 3,
    Error = 4,
}

impl Kind {
    fn from(b: u8) -> Option<Kind> {
        Some(match b {
            1 => Kind::Request,
            2 => Kind::Response,
            3 => Kind::Event,
            4 => Kind::Error,
            _ => return None,
        })
    }
}

/// Operations. Below 0x100 the kernel answers itself (works without userland); the rest go to the phone daemon.
pub mod op {
    pub const HELLO: u16 = 0x0001;
    pub const REBOOT: u16 = 0x0002;
    pub const LOG_START: u16 = 0x0003;
    pub const LOG_STOP: u16 = 0x0004;
    pub const STATUS: u16 = 0x0100;
    pub const METRICS: u16 = 0x0101;
    pub const RUN: u16 = 0x0102;
    pub const PUSH: u16 = 0x0103;
    pub const PULL: u16 = 0x0104;
    pub const SCREENSHOT: u16 = 0x0105;
    pub const MEDIA_IMPORT: u16 = 0x0106;

    pub fn name(op: u16) -> &'static str {
        match op {
            HELLO => "hello",
            REBOOT => "reboot",
            LOG_START => "log-start",
            LOG_STOP => "log-stop",
            STATUS => "status",
            METRICS => "metrics",
            RUN => "run",
            PUSH => "push",
            PULL => "pull",
            SCREENSHOT => "screenshot",
            MEDIA_IMPORT => "media-import",
            _ => "unknown",
        }
    }
    /// Handled by the kernel itself.
    pub fn is_kernel(op: u16) -> bool {
        op < 0x100
    }
}

/// Reboot modes the kernel accepts in a REBOOT body (bergamota_reboot.c names; "fastboot" = ABL bootloader).
pub const REBOOT_MODES: [&str; 5] = ["normal", "fastboot", "bergamota", "recovery", "poweroff"];

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub kind: Kind,
    pub flags: u8,
    pub op: u16,
    pub tag: u32,
    pub head: Value,
    pub body: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum ProtoError {
    BadMagic,
    BadKind(u8),
    TooLarge(usize),
    BadHead(String),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::BadMagic => write!(f, "berga: bad frame magic (not a berga device, or out of sync)"),
            ProtoError::BadKind(k) => write!(f, "berga: unknown frame kind {k}"),
            ProtoError::TooLarge(n) => write!(f, "berga: frame of {n} bytes exceeds {MAX_FRAME}"),
            ProtoError::BadHead(e) => write!(f, "berga: frame head is not JSON: {e}"),
        }
    }
}
impl std::error::Error for ProtoError {}

impl Frame {
    pub fn new(kind: Kind, op: u16, tag: u32, head: Value, body: Vec<u8>) -> Frame {
        Frame { kind, flags: 0, op, tag, head, body }
    }
    pub fn request(op: u16, tag: u32, head: Value, body: Vec<u8>) -> Frame {
        Frame::new(Kind::Request, op, tag, head, body)
    }
    pub fn error(op: u16, tag: u32, msg: &str) -> Frame {
        Frame::new(Kind::Error, op, tag, serde_json::json!({ "error": msg }), Vec::new())
    }
    /// The error text of an Error frame.
    pub fn error_text(&self) -> String {
        self.head.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string()
    }

    /// Wire bytes. PAD is set automatically when the frame length is a multiple of [`PACKET`].
    pub fn encode(&self) -> Result<Vec<u8>, ProtoError> {
        let head = if self.head.is_null() { Vec::new() } else { serde_json::to_vec(&self.head).map_err(|e| ProtoError::BadHead(e.to_string()))? };
        let len = HEADER_LEN + head.len() + self.body.len();
        let pad = len.is_multiple_of(PACKET);
        if len + pad as usize > MAX_FRAME {
            return Err(ProtoError::TooLarge(len + pad as usize));
        }
        let mut out = Vec::with_capacity(len + 1);
        out.extend_from_slice(MAGIC);
        out.push(self.kind as u8);
        out.push((self.flags & !FLAG_PAD) | if pad { FLAG_PAD } else { 0 });
        out.extend_from_slice(&self.op.to_le_bytes());
        out.extend_from_slice(&self.tag.to_le_bytes());
        out.extend_from_slice(&(head.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(&self.body);
        if pad {
            out.push(0);
        }
        Ok(out)
    }
}

/// Stream reassembly: USB transfers may carry part of a frame or several frames.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Next complete frame, if any. On a corrupt header the buffer is dropped (the caller reports it).
    #[allow(clippy::should_implement_trait)] // fallible: Iterator would hide the error
    pub fn next(&mut self) -> Result<Option<Frame>, ProtoError> {
        if self.buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let b = &self.buf;
        if &b[0..4] != MAGIC {
            self.buf.clear();
            return Err(ProtoError::BadMagic);
        }
        let kind = match Kind::from(b[4]) {
            Some(k) => k,
            None => {
                let k = b[4];
                self.buf.clear();
                return Err(ProtoError::BadKind(k));
            }
        };
        let flags = b[5];
        let op = u16::from_le_bytes([b[6], b[7]]);
        let tag = u32::from_le_bytes(b[8..12].try_into().unwrap());
        let hl = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let bl = u32::from_le_bytes(b[16..20].try_into().unwrap()) as usize;
        let total = HEADER_LEN.saturating_add(hl).saturating_add(bl).saturating_add((flags & FLAG_PAD) as usize);
        if total > MAX_FRAME {
            self.buf.clear();
            return Err(ProtoError::TooLarge(total));
        }
        if self.buf.len() < total {
            return Ok(None);
        }
        let head_bytes = &self.buf[HEADER_LEN..HEADER_LEN + hl];
        let head = if hl == 0 {
            Value::Null
        } else {
            match serde_json::from_slice(head_bytes) {
                Ok(v) => v,
                Err(e) => {
                    self.buf.drain(..total);
                    return Err(ProtoError::BadHead(e.to_string()));
                }
            }
        };
        let body = self.buf[HEADER_LEN + hl..HEADER_LEN + hl + bl].to_vec();
        self.buf.drain(..total);
        Ok(Some(Frame { kind, flags: flags & !FLAG_PAD, op, tag, head, body }))
    }

    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_split_and_joined() {
        let a = Frame::request(op::RUN, 7, json!({"cmd": "bergamotametrics"}), b"xyz".to_vec());
        let b = Frame::new(Kind::Event, op::LOG_START, 9, Value::Null, b"log line\n".to_vec());
        let mut wire = a.encode().unwrap();
        wire.extend(b.encode().unwrap());
        // byte at a time
        let mut d = Decoder::new();
        let mut got = Vec::new();
        for byte in &wire {
            d.push(std::slice::from_ref(byte));
            while let Some(f) = d.next().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, vec![a, b]);
        assert_eq!(d.pending_bytes(), 0);
    }

    #[test]
    fn pad_when_multiple_of_packet() {
        let f = Frame::request(op::PUSH, 1, Value::Null, vec![0xaa; PACKET - HEADER_LEN]);
        let w = f.encode().unwrap();
        assert_eq!(w.len(), PACKET + 1);
        assert_eq!(w[5] & FLAG_PAD, FLAG_PAD);
        let mut d = Decoder::new();
        d.push(&w);
        let back = d.next().unwrap().unwrap();
        assert_eq!(back.body.len(), PACKET - HEADER_LEN);
        assert_eq!(back.flags & FLAG_PAD, 0);
        assert_eq!(d.pending_bytes(), 0);
    }

    #[test]
    fn limits_and_garbage() {
        assert!(matches!(Frame::request(op::PUSH, 1, Value::Null, vec![0; MAX_FRAME]).encode(), Err(ProtoError::TooLarge(_))));
        assert!(Frame::request(op::PUSH, 1, json!({"path": "/var/mobile/Media/x"}), vec![0; CHUNK]).encode().is_ok());
        let mut d = Decoder::new();
        d.push(b"HTTP/1.1 200 OK\r\n\r\n....");
        assert_eq!(d.next(), Err(ProtoError::BadMagic));
        let mut big = Frame::request(op::PUSH, 1, Value::Null, Vec::new()).encode().unwrap();
        big[16..20].copy_from_slice(&(MAX_FRAME as u32).to_le_bytes());
        d.push(&big);
        assert!(matches!(d.next(), Err(ProtoError::TooLarge(_))));
    }
}
