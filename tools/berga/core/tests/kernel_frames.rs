//! Frames produced by the kernel's own berga code (xnu patch 1037, compiled on the host by
//! tools/berga/kernel-test/run.sh) decode with berga-core: HELLO's JSON head, the no-daemon error, a daemon
//! response, a PAD frame, the log stream's response and event, a reboot response.

use berga_core::client::{Client, Incoming};
use berga_core::proto::{Decoder, Kind, op};

#[test]
fn kernel_frames_decode() {
    let wire = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/kernel-frames.bin")).unwrap();
    let mut d = Decoder::new();
    d.push(&wire);
    let mut frames = Vec::new();
    while let Some(f) = d.next().unwrap() {
        frames.push(f);
    }
    assert_eq!(d.pending_bytes(), 0);
    let kinds: Vec<(Kind, u16, u32)> = frames.iter().map(|f| (f.kind, f.op, f.tag)).collect();
    assert_eq!(
        kinds,
        vec![
            (Kind::Response, op::HELLO, 7),
            (Kind::Error, op::STATUS, 8),
            (Kind::Response, op::RUN, 9),
            (Kind::Response, op::PULL, 11),
            (Kind::Response, op::LOG_START, 12),
            (Kind::Event, op::LOG_START, 12),
            (Kind::Response, op::REBOOT, 14),
        ]
    );
    assert_eq!(frames[0].head["proto"], 1);
    assert_eq!(frames[0].head["max_frame"], 65536);
    assert!(frames[1].error_text().contains("no daemon"));
    assert_eq!(frames[3].body.len(), 492);
    assert_eq!(frames[5].body.len(), 1000);
    assert_eq!(frames[6].head["mode"], "fastboot");
    // and through the client, as the CLI and the web page see them
    let mut c = Client::new();
    let got = c.feed(&wire);
    assert!(matches!(&got[0], Incoming::Reply { tag: 7, .. }));
    assert!(matches!(&got[5], Incoming::Event { tag: 12, .. }));
}
