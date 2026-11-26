# berga

One Rust codebase for managing the Bergamota phone (Poco F1 running iOS) from a computer: the `berga` CLI, the
desktop app (`apps/bergamota-gpui`, in progress) and the website's `/manage/` page all use `berga-core`. The phone
side is the kernel's berga USB interface (xnu patch 1037) plus the small daemon `bergad`. Protocol and USB details:
[docs/BERGA_USB.md](../../docs/BERGA_USB.md).

| crate | what |
| --- | --- |
| `core` (`berga-core`) | the protocol, fastboot, sparse images, boot image repack, the queue, the app's JSON files. Sans-IO; feature `usb` adds the nusb transports |
| `cli` (`berga`) | the command line; replaces `apps/Bergamota/scripts/bergamota` |
| `wasm` (`berga-wasm`) | `berga-core` for the browser: plain C ABI + JSON (no wasm-bindgen, like `tools/renegade-wasm`), incl. the fake phone |
| `phone` (`bergad`) | runs on the phone: answers what the kernel forwards to `/dev/berga` |
| `kernel-test` | compiles the kernel's berga code from the patch on the Mac and checks its frames |

## Build and test

```sh
cd tools/berga
cargo test --workspace                    # protocol framing, client, fake phone, fastboot, sparse, bootimg, queue, inbox
cargo build --release -p berga            # target/release/berga
kernel-test/run.sh                        # kernel 1037 code on the host (ASan/UBSan) + decode its frames
cargo build --release -p berga-wasm --target wasm32-unknown-unknown   # or apps/web: npm run build:wasm
phone/build.sh                            # bergad for the phone (rustup target add aarch64-apple-ios)
```

## CLI

`berga --help` lists everything. `--fake` swaps the phone for the built-in fake one.

```sh
berga devices                    # berga / fastboot interfaces on USB
berga --fake status              # phone status (berga USB), else the Mac app's status.json
berga metrics [--json]           # CPU/GPU/battery
berga log                        # live kernel log
berga run sysctl kern.bergamota_usb
berga screenshot -o shot.png
berga upload IMG_1234.HEIC       # into the camera roll
berga reboot bergamota           # normal | fastboot | bergamota | recovery | poweroff
berga queue                      # the boot/flash queue (queue.json)
berga boot out/boot.img --preset "default @3x"     # new boot-args into a copy of the image, then enqueue it
berga flash root.simg --then-boot out/boot.img     # userdata (default), boot or logo only
berga queue run                  # send queued items here when the Mac app is not running
```

Boots and flashes always go through the queue. While the Mac app runs, `enqueue`/`boot`/`flash`/`install`/`upload --app`
go to its inbox exactly like the old script (same JSON, same `--wait`); without the app, berga edits `queue.json`
itself (Swift-compatible) and `berga queue run` sends one item per fastboot appearance over nusb (boot items and full
flashes; delta flashes stay with the app). Files: `$BERGAMOTA_HOME` or `~/Library/Application Support/Bergamota`.

## Using berga-core from an app

Everything is plain structs, `serde_json::Value`s and an event enum; no UI types, no async runtime.

```rust
use berga_core::{session::Session, usb::UsbPipe, fake::FakeTransport, client::Incoming};

let mut s = Session::new(UsbPipe::berga()?);          // or Session::new(FakeTransport::default())
let hello = s.hello()?;                               // {"proto":1,"kernel":..,"daemon":true,..}
let metrics = s.metrics()?;                           // bergamotametrics JSON
s.log_start(16384)?;
while let Some(ev) = s.next_event(std::time::Duration::from_millis(500))? {
    if let Incoming::Event { body, .. } = ev { /* console text */ }
}
```

- `Session<T: Transport>` is blocking; run it on a worker thread and forward its results/events to the UI.
  Each call times out after 15 s.
- For an async or custom I/O loop use `client::Client` directly: `request()` gives bytes to send, `feed()` turns
  received bytes into `Incoming::{Reply, Failed, Event, TimedOut, Protocol}`, `expire(now)` applies the timeout.
- `usb::list()` enumerates berga and fastboot interfaces (for a device picker or the queue runner).
- `fastboot::Fastboot` (boot, full flash in sparse pieces), `queue::{Queue, Runner}` (queue.json and the
  one-send-per-fastboot rule), `appfiles` (status.json, inbox), `presets`, `bootimg`, `metrics::format_*`.
- `partition::check` is the hard allowlist (boot, userdata, logo); `fastboot::command` refuses everything else.

## Phone daemon

`bergad` must run on the phone for status/metrics/run/files/screenshots (the kernel answers hello, reboot and the
log itself). Not in the root image yet; by hand over SSH (see `phone/build.sh`):

```sh
phone/build.sh
scp -O target/aarch64-apple-ios/release/bergad root@192.168.7.2:/usr/local/bin/bergad
scp -O phone/launchd/org.bergamota.bergad.plist root@192.168.7.2:/System/Library/LaunchDaemons/
ssh root@192.168.7.2 launchctl load /System/Library/LaunchDaemons/org.bergamota.bergad.plist
```

## Not done yet

- Install of an IPA over berga USB (needs the IPA unpacked and pacstripped on the host: `renegade-rs` has zip and
  pacstrip; today `berga install` goes through the Mac app).
- Delta flash (`oem hash`) in `queue run`; the web flasher (`apps/web/src/lib/fastboot.ts`, `sparse.ts`) still uses
  its TypeScript fastboot/sparse code, `berga-core` has the same logic in Rust for a later switch.
- `bergad` in the Renegade root image and a launchd job there.
