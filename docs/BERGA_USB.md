# berga USB: the phone's management interface

The iOS kernel (Heartwood, xnu patch `1037-bergamota-berga-usb.patch`, both series) gives the phone a vendor USB
interface next to USB Ethernet. Host tools talk to it directly: the `berga` CLI and the desktop app through
`berga-core` (nusb), the website (`/manage/`) through WebUSB with the same `berga-core` compiled to wasm. There is no
server and no SSH in this path. Code: `tools/berga/` (README there).

## USB identity

| | |
| --- | --- |
| VID:PID | `1209:0001` (**placeholder**, see below) |
| Device class | EF/02/01 (composite with IAD), bcdUSB 2.10 |
| Interfaces | 0-1 CDC-ECM (USB Ethernet, unchanged), **2 berga: class ff, subclass 0x62, protocol 0x01**, string "Bergamota berga" |
| Endpoints (if 2) | bulk OUT 0x03, bulk IN 0x83, 512 bytes (high speed) |
| BOS | USB 2.0 extension (no LPM); WebUSB platform capability (vendor request 1, landing page `https://bergamota.zesmoi.com.br/manage/`); Microsoft OS 2.0 platform capability (vendor request 2, descriptor set 178 bytes) |
| MS OS 2.0 | function subset for interface 2: compatible ID `WINUSB`, `DeviceInterfaceGUIDs` = `{8C1E6F52-2B1D-4C6E-9A5F-7D3B0E4A1C29}` |

Hosts match the interface by its class triple (`ff/62/01`), never by VID/PID, so the IDs can change without touching
the tools beyond `berga_core::ids`.

**The VID/PID question (open, needs the user's decision).** `1209:0001` is one of pid.codes' *test* PIDs: free to
use while developing, not for anything distributed. The kernel has presented it since the USB-Ethernet bring-up.
Options:

1. Request a PID from pid.codes (VID 0x1209, free, a pull request to `pidcodes/pidcodes.github.com` with the
   project's URL and an open-source license). Their rules are written for open-source *hardware*; whether a kernel
   gadget on someone else's phone qualifies is their call. Proposed request: "Bergamota phone management
   interface", one PID.
2. Keep `1209:0001` for development only and decide before any public build.

Not an option: reusing another vendor's VID (Google's 18d1, which ABL and Enxerto's fastboot use, included).

## Platforms

- **macOS, ChromeOS:** nothing to install. Chrome/Edge may claim interface 2 (no kernel driver binds a vendor class).
- **Windows:** the MS OS 2.0 descriptors make Windows bind WinUSB to interface 2 by itself (Windows 8.1+). CDC-ECM
  has no in-box driver there; that only affects USB Ethernet.
- **Linux:** a udev rule for the browser (and non-root `berga`):
  `SUBSYSTEM=="usb", ATTR{idVendor}=="1209", ATTR{idProduct}=="0001", MODE="0660", TAG+="uaccess"`

## Frames

Every USB transfer carries whole frames; host and phone normally send exactly one frame per transfer.

```
off size
0   4   magic "BRG1"
4   1   kind      1 request, 2 response, 3 event, 4 error
5   1   flags     bit0 PAD: one 0x00 byte follows the body; bit1 MORE: more events follow for this tag
6   2   op
8   4   tag       chosen by the host per request, echoed by its response/error/events
12  4   head_len  JSON (UTF-8), 0 = none
16  4   body_len  raw bytes, 0 = none
20      head, body, [pad]
```

Little endian. A frame, pad included, is at most **65536 bytes** (the kernel's buffer per direction). A frame
whose length is a multiple of 512 gets the PAD byte, so every transfer ends in a short packet (no zero-length
packets needed on either side; WebUSB cannot send them reliably). Error frames have the head `{"error": "..."}`.
A corrupt header (bad magic, oversized lengths) makes the receiver drop what it buffered and resynchronise on
the next transfer.

**Timing:** the host gives up on a request after 15 s (`PHONE_TIMEOUT_MS`). File transfers are split into
60 KiB requests, so no single request runs long. A client should start with HELLO: it ends a log stream an
earlier client left running.

## Operations

Ops below 0x100 are answered by the **kernel** (they work while userland is dead or not started yet). Every other
request goes, as the whole frame, to **`/dev/berga`**, read by the phone daemon `bergad`
(`tools/berga/phone`); without it the kernel answers `error: no daemon`.

| op | name | request | response |
| --- | --- | --- | --- |
| 0x0001 | HELLO | - | head `{proto, kernel, uptime_ms, daemon, max_frame, usb{rx,tx,resets}}` |
| 0x0002 | REBOOT | body: `normal`, `fastboot` (ABL), `bergamota` (Enxerto fastboot), `recovery` or `poweroff` | head `{mode}`, then the phone reboots after 0.5 s (`kern.bergamota_reboot_mode` + `reboot_kernel`; poweroff = `RB_HALT` = PSCI SYSTEM_OFF, patch 1036) |
| 0x0003 | LOG_START | body: decimal backlog bytes (max 256 KiB) | head `{position}`; then events (op 0x0003, same tag, flag MORE, body = console text, up to 16 KiB each) |
| 0x0004 | LOG_STOP | - | empty response |
| 0x0100 | STATUS | - | head: iOS version/build, kernel, model, uptime, disks, `kern.bergamota_usb`/`_berga` |
| 0x0101 | METRICS | - | head: `bergamotametrics` JSON (docs/BERGAMOTA_METRICS.md) |
| 0x0102 | RUN | head `{cmd, args}` | head `{code, truncated}`, body = output (60 KiB max) |
| 0x0103 | PUSH | head `{path, offset, truncate, mode}`, body = data | head `{written}` |
| 0x0104 | PULL | head `{path, offset, len}` | head `{size, eof}`, body = data |
| 0x0105 | SCREENSHOT | - | head `{type}`, body = PNG (`bergamotashot`, every 3rd pixel) |
| 0x0106 | MEDIA_IMPORT | head `{path}` under `/var/mobile/Media/berga/` | head `{path}`: moved to `DCIM/100APPLE/IMG_NNNN.EXT`, owner mobile |

### Policy

The kernel only moves frames. The allowlists live in `berga_core::server` and are enforced twice: by the host
before sending (better errors) and by `bergad` on the phone whatever arrives:

- RUN: `bergamotametrics`, `bergamotashot`, `bergamotauicache` (`/usr/local/bin`), `sysctl` (read only: no `=`, no
  `-w`), `uptime`, `df`, `ps`. 15 s limit each. No shell.
- PUSH: `/var/mobile/Media/`, `/Applications/`, `/var/root/berga/`. PULL: those plus `/var/log/` and crash reports.
  Absolute paths, no `.`/`..`/empty components (`/private/var` = `/var`).
- REBOOT: the five modes above.
- Flashing is not a berga operation: boot images and partitions go through fastboot (Enxerto or ABL) with the
  boot/userdata/logo allowlist (`berga_core::partition`, the web flasher, Enxerto itself).

## Kernel side (patch 1037)

- Physical endpoints 6 (0x03 OUT) and 7 (0x83 IN, TX FIFO 3), configured with the ECM endpoints at
  SET_CONFIGURATION; a berga setup failure leaves USB Ethernet working (`kern.bergamota_usb_log` tag 9).
- DMA area grows from 16 KiB to 144 KiB: TRBs at 0x1060/0x1070, one 64 KiB buffer per direction.
- Two 256 KiB FIFOs of whole frames: host-bound (kernel frames + daemon writes) and daemon-bound. `ud_thread`
  (1 ms poll) starts the next host-bound frame and pumps the log stream (only when 16 KiB of FIFO is free).
- `/dev/berga` (root, mode 0600, one opener): `read()` returns exactly one request frame (buffer of 65536 bytes
  required); `write()` takes exactly one response/event/error frame, waits up to 2 s for FIFO room, then EAGAIN.
- Bus reset / disconnect: transfers ended, half-received frame dropped, log stream stopped, host-bound FIFO
  flushed.
- `kern.bergamota_berga`: `ready daemon log in out to_daemon drops txbusy txq rxq`. Boot-arg `bergamotaberga=0`:
  the interface is still described but has no backend (requests time out).

## Testing without the phone

- `tools/berga/kernel-test/run.sh`: cuts the berga code out of the patch, compiles it on the Mac against xnu stubs
  (ASan/UBSan) and drives it like the controller would; `berga-core` then decodes every frame the kernel produced.
- The fake phone (`berga_core::fake`) answers the whole protocol: `berga --fake ...`, the core tests, and the
  "Try the demo" button of `/manage/` (inside the wasm module).
