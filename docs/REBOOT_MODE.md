# Reboot to fastboot / recovery (Poco F1, kernel patch 0137)

Before 0137 the XNU port had no restart path at all (`PE_halt_restart` was never
set: "MACH Reboot" printed, then a spin until a watchdog or the user reset the
phone). 0137 adds one (`bsd/dev/sweet_reboot.c`, hooked in
`SweetPlatformExpert::haltRestart`) and a reboot reason.

## What the kernel does at reboot (all logged as `SWEET REBOOT:`)

1. PM8998 PON block, SID 0, `PON_SOFT_RB_SPARE` (0x88f): read-modify-write of
   bits 7:1 with `magic << 1` (Linux `qcom-pon.c`, pm8998 `reason_shift = 1`;
   `pm8998.dtsi` v6.12: `mode-bootloader = <0x2>`, `mode-recovery = <0x1>`).
   Every reboot writes the field (0 = normal) like the reboot-mode core, so a
   reason cannot stick. The write needs the PON APID to be owned by EE 0; if it
   is not, the log says `owned by EE n ... unavailable`.
2. IMEM `restart_reason` (0x146bf65c, live DT `qcom,msm-imem-restart_reason`):
   0x77665500 (bootloader) / 0x77665502 (recovery), non-normal modes only.
   The live DT has no `reboot-mode` node; downstream msm-poweroff writes both.
   (NOTES.md: IMEM alone did not select fastboot on SDM845, hence the PON byte.)
3. `PS_HOLD` (0x0c264000) = 0; after 2 s PSCI SYSTEM_RESET.

## Bergamota mode (kernel patch 0140)

`bergamota` reboots into **Bergamota fastboot**, the fastboot Enxerto, our bootloader
speaks (serial `bergamota0001`, `getvar version-bootloader` = `bergamota-1`),
instead of Qualcomm ABL fastboot. The kernel writes IMEM `restart_reason`
(0x146bf65c) = 0x42455247 (`BERG`) and the PON reason field = 0 (normal), so ABL
boots the boot partition normally, where Enxerto lives; Enxerto sees `BERG`,
clears the word and enters Bergamota fastboot. Needs 0140 on top of 0137 and a
Enxerto that handles `BERG`. `sweetreboot bergamota` (and `sweetreboot -n bergamota`).
Panics are unaffected (`sweetreboot=bergamota` as boot-arg only sets the default mode).

## Control

- Boot-arg `sweetreboot=bootloader|recovery|bergamota|normal` (default `normal`).
- `sysctl kern.sweet_reboot_mode` (root, string) overrides it for the next
  reboot: `bootloader` (alias `fastboot`), `recovery`, `bergamota`, `normal`.
- Panic: unchanged (the phone stays up on the panic screen) unless
  `sweetreboot=bootloader` (reset into fastboot) or `sweetpanicreboot=1`
  (patch 1020: reset in normal mode, ABL boots `boot` = iOS again) is a
  boot-arg. The sysctl does not affect panics.

## Unattended recovery (kernel patch 1020)

- **Panic -> reset.** With `sweetpanicreboot=1` the reset happens at
  `kPEPanicDiagnosticsDone`, which every panic reaches (with `debug=0xc`,
  DB_NMI sets `panicDebugging`, so XNU never asks for `kPEPanicRestartCPU` and
  used to spin forever). Before the reset `sweet_ufs_flush_final()` writes the
  console ring's unwritten tail (the panic text) to the log area.
- **Previous boot's log.** At boot the log thread reads the previous boot's last
  8 KiB back before its first flush overwrites the ring, prints it as `prev| ...`
  lines (between `SWEET LOG: previous boot's last ...` and `SWEET LOG: end of the
  previous boot's log`) and keeps it in `sysctl kern.sweet_log_prev`. Over SSH:
  `/var/root/sysctlget kern.sweet_log_prev` (prints hex; `xxd -r -p` on the Mac).
- **Hardware watchdog.** `sweetwdt=N` (5..120 s) arms the APSS WDT
  (0x17980000, 32768 Hz, Linux qcom-wdt kpss offsets) with a bite after N s and
  pets it from a kernel thread every N/4 s; the bark IRQ stays off. A wedged
  kernel, or a panic that never reaches the reset, gets bitten. Halt disables it.
  Boot log: `SWEET REBOOT: watchdog armed, bite after N s ...`.
- **Tests.** `sweetreboot test-panic` (`kern.sweet_reboot_test=1`, a panic) and
  `sweetreboot test-hang` (`=2`: watchdog pets stopped, the calling CPU spins
  with interrupts off; needs `sweetwdt`).
- `tools/renegade/mkboot-full.sh` adds `sweetpanicreboot=1 sweetwdt=30` by default.
- **Mac fallback.** `tools/fastboot-rescue.sh` (self-check
  `tools/test-fastboot-rescue.sh`): when the phone sits in ABL fastboot for 90 s
  (not Bergamota fastboot, and not while the Bergamota app has its queue armed) it
  runs `fastboot reboot`, at most 3 times an hour. Run it with `nohup`; log in
  `logs/fastboot-rescue.log`.

## Tool

`tools/sweetnet/sweetreboot.c` (built by `tools/sweetnet/build.sh` into
`research/sweetnet/sweetreboot`, ad-hoc signed): `sweetreboot fastboot|recovery|bergamota|normal`
sets the sysctl, syncs and calls `reboot(RB_AUTOBOOT)`; `-n` only sets the mode.

Install on a running phone (no image rebuild):

    scp research/sweetnet/sweetreboot root@192.168.7.2:/usr/local/bin/sweetreboot
    ssh root@192.168.7.2 'chmod 755 /usr/local/bin/sweetreboot'

New images get it from `tools/sweet-root-tweaks.py` (like `sweetnetcfg`).
The kernel must contain 0137 (the tool refuses to reboot without the sysctl).

## First test, in order

1. Build the kernel with 0137 (CI), boot it; in the log check
   `SWEET REBOOT: boot-arg mode normal, PS_HOLD mapped, IMEM mapped, PON spare write ready`.
2. Install the tool (above); `ssh root@192.168.7.2 sysctl kern.sweet_reboot_mode`.
3. `ssh root@192.168.7.2 sweetreboot -n fastboot`, check the sysctl, then
   `sweetreboot fastboot`. Expect fastboot; if Android boots instead, pull the
   console log and read the `SWEET REBOOT:` lines (PON write refused vs. ABL
   ignoring the byte).
4. `sweetreboot normal` must reboot into Android's ABL path as before (no reason set).
