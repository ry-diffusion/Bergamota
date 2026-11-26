# Study: Jesvi iOS on the Galaxy S20 (2026-11-26)

Source: https://github.com/jesvi-eulah/Jesvi-iOS (read at 6e33d46). It runs the iOS 16.5 (20F66) iPhone X userland on the Exynos 990 Galaxy S20 with XNU 8796.121.2, its own UEFI loader, drivers and services. It started on an Adreno phone (Poco X5 Pro "redwood", from HakiOS), so parts of it are Qualcomm. Paths below are in that repo.

**Licence:** kernel files are APSL 2.0 (fine in our XNU fork if modified files stay published with their notices). Most of the rest is MIT (attribution only). Avoid `wifi/lx/include/linux` (partly GPL-2.0) and `tools/cellular/s5123-*` (GPL-2.0-only). Credit Jesvi iOS and HakiOS when reusing.

## What to take, ranked

1. **Battery widget "Waiting…" and status-bar battery: a kernel `IOPMPowerSource` subclass** (`xnu/files/iokit/Kernel/RedwoodBattery.cpp`).
   - Set the properties before `registerService()`, attach the service under the platform expert, and call `updateStatus()` on each reading.
   - Properties: BatteryInstalled, ExternalConnected, ExternalChargeCapable, IsCharging, FullyCharged, AppleRawExternalConnected; CurrentCapacity % with MaxCapacity 100 (powerd needs both); Voltage, Amperage, InstantAmperage, Temperature (0.01 °C), CycleCount; AppleRawCurrentCapacity, AppleRawMaxCapacity, NominalChargeCapacity, DesignCapacity; AdapterDetails (Watts, Current, Voltage, FamilyCode, e.g. 0xe000400a for USB PD, IsWireless; FamilyCode 0 with no charger).
   - Without this service, IOPowerSources is empty.
2. **Charging stops about a minute after boot: the charger chip's watchdog.** On the S20 (MAX77705) nobody petted it under iOS (`bms_wdt=1` pets it each poll). Check the PMI8998 charger's watchdog (Linux `qcom,pmi8998-charger` / smb2) on the Poco F1. Alternative from their Qualcomm path: keep the UEFI charger DXE alive and only read its state (`xnuloader/loader.c` ~1209-1430).
3. **Lock-screen clock missing: the digits are a vibrancy mask the CA software renderer draws wrongly.** The `BSUIVibrancyEffectViewDisabled` user default makes it plain white (`userland/src/inject_sb.c` ~819). Also: restore the later of the saved time and the image build time before launchd (`userland/src/init.c` clock_restore), plus SNTP (`clockd.c`). A 1970 clock sent powerlog to 92 % CPU.
4. **Wi-Fi on iOS 18: the same design as our iOS 26 stand-in, so it confirms it.**
   - Property-only IOKit classes named after Apple's: `IO80211Controller` with a child `IONetworkInterface` that has `IO80211InterfaceRole=Infrastructure` and `IOInterfaceName=en0`.
   - `newUserClient` fails on purpose, so IO80211.framework falls back to `SIOCGA80211/SIOCSA80211` ioctls on en0. Events go out as `KEV_IEEE80211_CLASS` kernel events. Stock wifid runs unmodified.
   - Own WPA2 supplicant (`wifi/lx/src/lx_wpa.c`, MIT, with tests in `tools/wpa-test`), plus an AP-side authenticator for Personal Hotspot.
   - Their iOS 16.5 layouts are pinned with `_Static_assert` in `xnu/files/bsd/dev/redwood_apple80211.c` (ASSOCIATE 592 B with the key at 0x48; SCAN_REQ 5456; SCAN_RESULT 1164; CHANNELS_INFO 10016; CARD_CAPABILITIES bits 2,3,6,7,9,10,14,17,19, where wrong bits mean "Unable to join").
   - iOS 18 layouts have to be read from 22H352's IO80211.framework (`_performAssociation`, `_getScanData`, ...). The request numbers are probably stable.
   - `userland/mkjobs.py` drops wifid's `_LimitLoadToDeviceTree` (it wants a "wlan" DT node). DHCP must come from configd's own service, or wifid treats the interface as unusable.
5. **Panic, halt and reboot go back to the bootloader with the log ring and panic buffer**, which saves them to flash; a hypervisor watchdog resets any hang. We now have 1020 (panic reset + APSS watchdog + previous-boot console) and 1036 (PSCI power-off).
6. **First boot:** SpringBoard activation state forced to 2 and SBSetupManager refused, a seeded purplebuddy plist (SetupDone, ForceNoBuddy), and the MobileGestalt cache invalidated when hardware flags change. TrollStore installed once per data volume.
7. **GPU (different from Crimson):**
   - backboardd's CoreAnimation draws with **OpenGL ES** (Mesa Gallium, replaced EAGLContext); Metal is kept out of it.
   - Metal is a real Metal plugin bundle (`RedwoodMetalDevice : _MTLDevice`, found through `MetalPluginName` on IOAcceleratorES) over their own AIR to NIR compiler (no libLLVM) and Gallium. It is for apps and a few posters only.
   - Unverified for us: whether iOS 18 QuartzCore still has a GLES renderer.
   - Reusable for Turnip: AIR to NIR straight into ir3 (skip SPIR-V); their ir3 fixes (`ir3-round-default`, `ir3-shift-split`, `ir3-ssbo-subdword`); highp for CA's mediump shaders (banding on Adreno); `EXT_shader_framebuffer_fetch` (about 4x fewer submissions); completion queues at USER_INTERACTIVE QoS (chronod handlers waited 2-7 s); half-size blur chains; a frame-floor CPU/GPU clock while frames flow.
   - Per-process Metal discovery is filtered in the kernel (`redwood_graphics_policy.h`), an alternative to our switch files.
   - PosterBoard stays off Metal (scrambled bold titles). Collections and Weather posters crash with it.
   - Numbers: 10.3 ms median CA frame, about 75 fps; the render thread spends 13 % in IOKit (IOSurface imports).
8. **Display:** vsync is a timer phase-locked to the panel's TE. 120 Hz needs TimingElements 120 + `CA_HEARTBEAT_RATE=120` for backboardd + touch scanning at 120. The idle timer never fires on their phone, so an injected lock-button press does screen sleep.
9. **Sensors, haptics, audio:** a kernel `/dev` node plus user-space adapters (sensor samples as HID vendor events injected into backboardd; a CoreAudio HAL plug-in for speaker, mic and haptics).
10. **Dev loop:** one cmdline profile per device, features as boot-args, a baseline-and-delta workflow for kernel patches (`tools/kernel-delta.sh`), host unit tests for policy headers (`ci/check.sh`).

Not reusable: their Apple ID path (Anisette from a companion Hackintosh), the Mali/Exynos drivers, and their hooks at fixed iOS 16.5 addresses (use selector names on 18.7.8).
