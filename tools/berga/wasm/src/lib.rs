//! berga-core in the browser: a plain C ABI (no wasm-bindgen, like tools/renegade-wasm). The page's JavaScript
//! moves the bytes over WebUSB; this module encodes requests, decodes replies/events, applies the timeouts, and
//! runs the fake phone for the demo mode. One client and one fake phone per module instance.
//!
//! Inputs are (ptr, len) pairs from `bg_alloc`. Outputs land in a result buffer (`bg_result_ptr/len`): JSON text
//! or raw bytes, as each function says. Return values: >= 0 ok (count/tag), < 0 error (result = message).

use berga_core::client::{Client, Incoming};
use berga_core::fake::FakePhone;
use berga_core::{ids, proto, server};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;

thread_local! {
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static CLIENT: RefCell<Client> = RefCell::new(Client::new());
    static INCOMING: RefCell<VecDeque<Incoming>> = const { RefCell::new(VecDeque::new()) };
    static BODY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static FAKE: RefCell<FakePhone> = RefCell::new(FakePhone::new());
}

fn set_result(b: Vec<u8>) {
    RESULT.with(|r| *r.borrow_mut() = b);
}

fn err(msg: impl ToString) -> i32 {
    set_result(msg.to_string().into_bytes());
    -1
}

/// # Safety
/// `p` must come from `bg_alloc` with at least `n` bytes.
unsafe fn input<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if n == 0 { &[] } else { unsafe { std::slice::from_raw_parts(p, n) } }
}

#[unsafe(no_mangle)]
pub extern "C" fn bg_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p` must come from `bg_alloc(n)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_free(p: *mut u8, n: usize) {
    drop(unsafe { Vec::from_raw_parts(p, 0, n.max(1)) });
}

#[unsafe(no_mangle)]
pub extern "C" fn bg_result_ptr() -> *const u8 {
    RESULT.with(|r| r.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn bg_result_len() -> usize {
    RESULT.with(|r| r.borrow().len())
}

/// Constants for the page (JSON): USB filter, limits, allowlists.
#[unsafe(no_mangle)]
pub extern "C" fn bg_info() -> i32 {
    set_result(
        json!({
            "vid": ids::VID, "pid": ids::PID,
            "class": ids::IFACE_CLASS, "subclass": ids::IFACE_SUBCLASS, "protocol": ids::IFACE_PROTOCOL,
            "maxFrame": proto::MAX_FRAME, "chunk": proto::CHUNK, "timeoutMs": berga_core::PHONE_TIMEOUT_MS,
            "rebootModes": proto::REBOOT_MODES, "run": server::RUN_ALLOWED, "partitions": berga_core::partition::ALLOWED,
        })
        .to_string()
        .into_bytes(),
    );
    0
}

/// Starts over (new connection): forgets pending requests and buffered bytes.
#[unsafe(no_mangle)]
pub extern "C" fn bg_reset() {
    CLIENT.with(|c| *c.borrow_mut() = Client::new());
    INCOMING.with(|i| i.borrow_mut().clear());
}

/// A request. Head: JSON text (empty = none). Returns the tag; result = the bytes for one transferOut.
/// # Safety
/// Pointers from `bg_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_request(op: u32, hp: *const u8, hn: usize, bp: *const u8, bn: usize, now_ms: f64) -> i32 {
    let head = unsafe { input(hp, hn) };
    let head: Value = if head.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(head) {
            Ok(v) => v,
            Err(e) => return err(format!("head: {e}")),
        }
    };
    let body = unsafe { input(bp, bn) }.to_vec();
    match CLIENT.with(|c| c.borrow_mut().request(op as u16, head, body, now_ms as u64)) {
        Ok((tag, bytes)) => {
            set_result(bytes);
            tag as i32
        }
        Err(e) => err(e),
    }
}

/// Bytes of one transferIn. Returns how many incoming items are now queued (take them with `bg_next`).
/// # Safety
/// Pointer from `bg_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_feed(p: *const u8, n: usize) -> i32 {
    let bytes = unsafe { input(p, n) };
    let got = CLIENT.with(|c| c.borrow_mut().feed(bytes));
    INCOMING.with(|i| {
        let mut q = i.borrow_mut();
        q.extend(got);
        q.len() as i32
    })
}

/// Requests past their 15 s deadline become TimedOut items. Returns the queue length.
#[unsafe(no_mangle)]
pub extern "C" fn bg_expire(now_ms: f64) -> i32 {
    let got = CLIENT.with(|c| c.borrow_mut().expire(now_ms as u64));
    INCOMING.with(|i| {
        let mut q = i.borrow_mut();
        q.extend(got);
        q.len() as i32
    })
}

/// Next incoming item: 1 and result = JSON {kind, tag, op, head?, error?}, its body in `bg_body_ptr/len`; 0 = none.
#[unsafe(no_mangle)]
pub extern "C" fn bg_next() -> i32 {
    let Some(item) = INCOMING.with(|i| i.borrow_mut().pop_front()) else { return 0 };
    let (meta, body) = match item {
        Incoming::Reply { tag, op, head, body } => (json!({ "kind": "reply", "tag": tag, "op": op, "head": head }), body),
        Incoming::Failed { tag, op, error } => (json!({ "kind": "failed", "tag": tag, "op": op, "error": error }), vec![]),
        Incoming::Event { tag, op, head, body } => (json!({ "kind": "event", "tag": tag, "op": op, "head": head }), body),
        Incoming::TimedOut { tag, op } => (json!({ "kind": "timeout", "tag": tag, "op": op }), vec![]),
        Incoming::Protocol(e) => (json!({ "kind": "protocol", "error": e }), vec![]),
    };
    BODY.with(|b| *b.borrow_mut() = body);
    set_result(meta.to_string().into_bytes());
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn bg_body_ptr() -> *const u8 {
    BODY.with(|b| b.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn bg_body_len() -> usize {
    BODY.with(|b| b.borrow().len())
}

/// Host-side checks before sending (same lists the phone enforces). 0 ok, -1 refused (result = why).
/// kind 0: push path, 1: pull path, 2: run (input JSON {"cmd","args"}).
/// # Safety
/// Pointer from `bg_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_check(kind: u32, p: *const u8, n: usize) -> i32 {
    let s = String::from_utf8_lossy(unsafe { input(p, n) }).to_string();
    let r = match kind {
        0 => server::path_allowed(&s, &server::PUSH_ROOTS).map(|_| ()),
        1 => server::path_allowed(&s, &server::PULL_ROOTS).map(|_| ()),
        2 => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()).and_then(|v| {
            let args: Vec<String> = v["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            server::run_allowed(v["cmd"].as_str().unwrap_or(""), &args)
        }),
        _ => Err("unknown check".into()),
    };
    match r {
        Ok(()) => 0,
        Err(e) => err(e),
    }
}

// ---- the fake phone (demo mode): the page feeds it the client's bytes and reads its transfers back ----

#[unsafe(no_mangle)]
pub extern "C" fn bg_fake_reset() {
    FAKE.with(|f| *f.borrow_mut() = FakePhone::new());
}

/// # Safety
/// Pointer from `bg_alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_fake_feed(p: *const u8, n: usize) {
    let bytes = unsafe { input(p, n) };
    FAKE.with(|f| f.borrow_mut().feed(bytes));
}

#[unsafe(no_mangle)]
pub extern "C" fn bg_fake_tick(now_ms: f64) {
    FAKE.with(|f| f.borrow_mut().tick(now_ms as u64));
}

/// 1 and result = one IN transfer of the fake phone, 0 = nothing to send.
#[unsafe(no_mangle)]
pub extern "C" fn bg_fake_take() -> i32 {
    match FAKE.with(|f| f.borrow_mut().take()) {
        Some(b) => {
            set_result(b);
            1
        }
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> Vec<u8> {
        RESULT.with(|r| r.borrow().clone())
    }

    #[test]
    fn abi_roundtrip_with_fake() {
        let head = br#"{"cmd":"uptime"}"#;
        let tag = unsafe { bg_request(proto::op::RUN as u32, head.as_ptr(), head.len(), std::ptr::null(), 0, 0.0) };
        assert!(tag > 0);
        let out = result();
        unsafe { bg_fake_feed(out.as_ptr(), out.len()) };
        assert_eq!(bg_fake_take(), 1);
        let t = result();
        assert_eq!(unsafe { bg_feed(t.as_ptr(), t.len()) }, 1);
        assert_eq!(bg_next(), 1);
        let meta: Value = serde_json::from_slice(&result()).unwrap();
        assert_eq!(meta["kind"], "reply");
        assert_eq!(meta["tag"], tag);
        assert!(BODY.with(|b| String::from_utf8_lossy(&b.borrow()).contains("up")));
        let bad = br#"{"cmd":"sh"}"#;
        assert_eq!(unsafe { bg_check(2, bad.as_ptr(), bad.len()) }, -1);
        assert_eq!(bg_expire(1e9), 0);
    }
}
