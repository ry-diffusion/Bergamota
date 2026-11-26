//! Sans-IO client: turns requests into bytes and incoming bytes into replies and events.
//! The caller moves the bytes (nusb natively, WebUSB in the browser) and supplies the clock.

use crate::PHONE_TIMEOUT_MS;
use crate::proto::{Decoder, Frame, Kind, ProtoError};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// The answer to request `tag`.
    Reply { tag: u32, op: u16, head: Value, body: Vec<u8> },
    /// The phone refused or failed request `tag`.
    Failed { tag: u32, op: u16, error: String },
    /// Unsolicited data for a subscription started by request `tag` (log stream).
    Event { tag: u32, op: u16, head: Value, body: Vec<u8> },
    /// No reply to `tag` within the timeout.
    TimedOut { tag: u32, op: u16 },
    /// The byte stream was corrupt (the decoder resynchronises on the next transfer).
    Protocol(String),
}

pub struct Client {
    next_tag: u32,
    dec: Decoder,
    /// tag, op, deadline (ms)
    pending: Vec<(u32, u16, u64)>,
    pub timeout_ms: u64,
}

impl Default for Client {
    fn default() -> Self {
        Client { next_tag: 1, dec: Decoder::new(), pending: Vec::new(), timeout_ms: PHONE_TIMEOUT_MS }
    }
}

impl Client {
    pub fn new() -> Client {
        Client::default()
    }

    /// Encodes a request; returns its tag and the bytes to send as one USB transfer.
    pub fn request(&mut self, op: u16, head: Value, body: Vec<u8>, now_ms: u64) -> Result<(u32, Vec<u8>), ProtoError> {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1).max(1);
        let bytes = Frame::request(op, tag, head, body).encode()?;
        self.pending.push((tag, op, now_ms + self.timeout_ms));
        Ok((tag, bytes))
    }

    pub fn is_pending(&self, tag: u32) -> bool {
        self.pending.iter().any(|p| p.0 == tag)
    }

    /// Bytes from one IN transfer.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Incoming> {
        self.dec.push(bytes);
        let mut out = Vec::new();
        loop {
            match self.dec.next() {
                Ok(Some(f)) => {
                    let done = |c: &mut Client, tag| c.pending.retain(|p| p.0 != tag);
                    match f.kind {
                        Kind::Response => {
                            done(self, f.tag);
                            out.push(Incoming::Reply { tag: f.tag, op: f.op, head: f.head, body: f.body })
                        }
                        Kind::Error => {
                            done(self, f.tag);
                            let error = f.error_text();
                            out.push(Incoming::Failed { tag: f.tag, op: f.op, error })
                        }
                        Kind::Event => out.push(Incoming::Event { tag: f.tag, op: f.op, head: f.head, body: f.body }),
                        Kind::Request => out.push(Incoming::Protocol("the phone sent a request frame".into())),
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    out.push(Incoming::Protocol(e.to_string()));
                    break;
                }
            }
        }
        out
    }

    /// Requests whose deadline passed; they are forgotten (a late reply is then ignored by the caller).
    pub fn expire(&mut self, now_ms: u64) -> Vec<Incoming> {
        let mut out = Vec::new();
        self.pending.retain(|&(tag, op, deadline)| {
            if now_ms >= deadline {
                out.push(Incoming::TimedOut { tag, op });
                false
            } else {
                true
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::op;
    use serde_json::json;

    #[test]
    fn reply_error_event_timeout() {
        let mut c = Client::new();
        let (t1, _) = c.request(op::HELLO, Value::Null, vec![], 0).unwrap();
        let (t2, _) = c.request(op::RUN, json!({"cmd":"rm"}), vec![], 0).unwrap();
        let (t3, _) = c.request(op::STATUS, Value::Null, vec![], 1000).unwrap();
        assert_ne!(t1, t2);
        let mut wire = Frame::new(Kind::Response, op::HELLO, t1, json!({"proto":1}), vec![]).encode().unwrap();
        wire.extend(Frame::error(op::RUN, t2, "not allowed").encode().unwrap());
        wire.extend(Frame::new(Kind::Event, op::LOG_START, 99, Value::Null, b"x".to_vec()).encode().unwrap());
        let got = c.feed(&wire);
        assert_eq!(got.len(), 3);
        assert!(matches!(&got[0], Incoming::Reply { tag, head, .. } if *tag == t1 && head["proto"] == 1));
        assert!(matches!(&got[1], Incoming::Failed { error, .. } if error == "not allowed"));
        assert!(matches!(&got[2], Incoming::Event { tag: 99, .. }));
        assert!(c.expire(PHONE_TIMEOUT_MS + 999).is_empty());
        assert_eq!(c.expire(PHONE_TIMEOUT_MS + 1000), vec![Incoming::TimedOut { tag: t3, op: op::STATUS }]);
        assert!(!c.is_pending(t3));
    }
}
