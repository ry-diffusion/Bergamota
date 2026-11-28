# Sensors (Poco F1, iOS 18.7.8): /dev/bergamota-sensors -> backboardd bridge -> iOS

Ported from the Galaxy S20 port **Jesvi iOS / HakiOS** (`xnu/files/bsd/{sys,dev}/redwood_sensors.*` under APSL 2.0,
`userland/src/inject_sensors.c` + `brightness_policy.h` under MIT, which itself came from Scion's S9 work), renamed
`bergamota`. See `docs/STUDY_JESVI_S20.md` item 9. Nothing here has run on the phone yet.

## Design

```
sensor driver (none yet) ----bergamota_sensors_post()---+
sensorpost (root, test path) --write(2)-----------------+--> queue (256 samples, bsd/dev/bergamota_sensors.c)
                                                              |  read(2)
                                       backboardd: bergamota-sensorbridge.dylib (tools/bergamotanet/sensorbridge.c)
                                          |-- virtual HID services on vendor page 0xFF00 (own IOHIDEventSystemClient)
                                          |      accel 0x03, ALS 0x04, proximity 0x08, gyro 0x09, mag 0x0A
                                          |-- CMDeviceOrientationManager methods replaced (rotation)
                                          '-- optional auto-brightness thread
```

* Kernel (`xnu/patches-11215/1136-bergamota-sensors-device.patch`): `bsd/dev/bergamota_sensors.c`, `bsd/sys/bergamota_sensors.h`,
  started from `bergamota_ufs.c` next to the other drivers. `struct bergamota_sensor_sample` (40 bytes: `time`, `hub_ns`,
  `type`, `scale`, `v[4]`), types ACCEL/GYRO/MAG/PRESSURE/LIGHT/PROXIMITY, Android axes (face up z = +1 g), same
  semantics as redwood's (documented in the header). `bergamota_sensors_post()` is for drivers (thread context only).
  The queue fills only while a reader (`FREAD`) has the device open; one reader, since each sample goes to one `read()`.
  Differences from redwood: the device is made by `bergamota_sensors_init()` from the boot-arg, not by the first driver;
  a write-only open does not count as a reader; no proximity-emitter ioctl (no hub yet).
* Boot-arg `bergamotasensors` (bit mask): **1** makes `/dev/bergamota-sensors` (0666), **2** lets root `write()` whole
  samples into it (test path; a write is refused with ENODEV without bit 2, EPERM for non-root, EINVAL for a size that is
  not a multiple of 40 or an unknown type; `time == 0` is filled in with `mach_absolute_time()`). Default 0: no device, nothing
  changes. Use `bergamotasensors=3` for testing, `=1` once a driver exists.
* Bridge (`tools/bergamotanet/sensorbridge.c`, built as `research/bergamotanet/ios18/bergamota-sensorbridge.dylib`): a
  dylib linked into `/usr/libexec/backboardd` (extra `LC_LOAD_DYLIB`, see Install). Its constructor runs only in
  `backboardd`, only if `/dev/bergamota-sensors` opens and `/private/var/tmp/sensorbridge.off` is absent; it replaces
  CMDeviceOrientationManager's methods, then a thread makes the HID client and services (retrying every 3 s until backboardd's
  event server answers) and turns samples into events. Log: `/private/var/tmp/sensorbridge.log`.
* Why a linked dylib and not `DYLD_INSERT_LIBRARIES`: backboardd's launchd job already runs `bgl-launch`, which in Metal mode does
  `setenv(DYLD_INSERT_LIBRARIES, crimson-inject, 1)` (overwrites), and launchd drops `DYLD_*` from the job anyway. The
  bridge therefore does not depend on Crimson or on the launcher. backboardd (22H352) has 120 bytes of load-command slack even
  with Crimson's inject linked (that one is 72 bytes, the bridge's 64), measured on the image's binary.
* Test tool: `sensorpost` (`tools/bergamotanet/sensorpost.c`).

## Install (nothing below was run by the author; the phone was not touched)

On the Mac, `tools/bergamotanet/build.sh` (add `TOOLS=" "` to build only the sensor tools) writes
`research/bergamotanet/ios18/{bergamota-sensorbridge.dylib,sensorpost}`.

**New root image / `renegade update`:** `RENEGADE_SENSORBRIDGE=1 renegade update ...` runs `root_tweaks18.sensorbridge()` (installs the dylib first,
then links and re-signs backboardd keeping identifier, flags and all 105 entitlements; checked). Off by default.

**Live phone** (SSH, the iOS 18 `scp -O` rule; backboardd is replaced as a file, the running one is untouched until it restarts):
```
cd <repo>; P=root@192.168.7.2
scp -O research/bergamotanet/ios18/bergamota-sensorbridge.dylib $P:/usr/local/lib/       # first
scp -O research/bergamotanet/ios18/sensorpost $P:/usr/local/bin/
scp -O $P:/usr/libexec/backboardd /tmp/backboardd.orig && ssh $P cp /usr/libexec/backboardd /var/root/backboardd.pre-sensors
python3 -I tools/renegade/root_tweaks18.py --link-dylib /tmp/backboardd.orig /usr/local/lib/bergamota-sensorbridge.dylib /tmp/backboardd.sb
scp -O /tmp/backboardd.sb $P:/usr/libexec/backboardd.new && ssh $P 'chmod 755 /usr/libexec/backboardd.new && mv /usr/libexec/backboardd.new /usr/libexec/backboardd'
```
Undo: `touch /private/var/tmp/sensorbridge.off` (the dylib stays linked but does nothing) or move `backboardd.pre-sensors` back
(keep the dylib installed: the original does not need it; the patched one does). Boot a kernel with patch 1136
(`research/ios18/kernel/kernel.development.bergamota-1136`, built from the local tree) and add `bergamotasensors=3` to the
boot-args, then respring/reboot so backboardd starts with the bridge.

## Test with sensorpost (as root, bergamotasensors=3)

```
cat /private/var/tmp/sensorbridge.log                # "bridge starting", "6 of CMDeviceOrientationManager's methods replaced",
                                                     # then "services: light 0x.. accelerometer 0x.. ..." (all non-zero)
sensorpost rotate portrait; sensorpost rotate left   # log: "orientation 1 (portrait)", "orientation 3 (home button right)"; SpringBoard rotates
sensorpost rotate right | upside | faceup | facedown
sensorpost prox 1; sensorpost prox 0                 # log: "proximity near/far"; with a call-style client the screen goes off
sensorpost lux 5 8; sensorpost lux 800 8             # ALS stream: CoreBrightness logs "ALS service ..." (logstream CoreBrightness)
touch /private/var/tmp/sensorbridge.autobright       # (before the boot) the opt-in auto-brightness thread; then Settings > Display > Auto-Brightness
```
`sensorpost accel X Y Z [N]` posts raw g values (Android axes). `sensorpost` needs the kernel's bit 2; `Operation not permitted` = not root,
`No such device` = no bit 2.

## What differs from iOS 16.5 (checked in the 22H352 shared cache, `tools/renegade/dsctools`)

* **Same, verified:** `IOHIDVirtualServiceClientCreateWithCallbacks(client, attributes, callbacks, target, refcon)` and the version-2 callback
  struct (version + six callbacks, 0x38 bytes copied); `IOHIDVirtualServiceClientDispatchEvent/Remove`, `IOHIDEventSetIntegerValue/FloatValue`,
  `IOHIDEventCreate{Accelerometer,Gyro,Compass,AmbientLightSensor,Proximty}Event` all exported by IOKit; event types accel 13 (0xd),
  ALS 12, proximity 14, gyro 20 (0x14), compass 21 (0x15); the proximity event keeps its mask in a 16-bit field; ALS field `0xc000b`
  is handled by IOKit's event field accessors and referenced by CoreBrightness. CoreBrightness still logs "ALS service %p: builtIn=%u location=%lu",
  accepts a service only built-in with VendorID 0x5ac (`subs w8, w8, #0x5ac` at CoreBrightness+0xb75ec), reads `ALSCurveInfo`
  ("Failed to obtain ALS Curve info"), and writes through `BrightnessSystemClient` `copyPropertyForKey:` / `setProperty:forKey:` with the keys
  `DisplayBrightnessAuto`, `DisplayBrightness`, `DisplayOn`, `DisplayNitsKey`, `Nits`, `Brightness`, `Commit`.
* **New in iOS 18: a virtual service needs a *scheduled* client.** `IOHIDVirtualServiceClientCreateWithCallbacks` logs
  "Client must be scheduled with dispatch queue or runloop" and returns NULL if the client has neither (IOKit+0x15d98 in
  `_IOHIDVirtualServiceClientCreateWithCallbacks`: it checks client+0x180 and +0x190). The S20 code reused backboardd's touch-input client, which is scheduled; we have a kernel HID touch
  driver and no such client, so the bridge creates its own: `IOHIDEventSystemClientCreate`, `...SetDispatchQueue`, `...Activate`
  (all exported).
* **Server-side check (iOS 18, backboardd is the server: its job owns `com.apple.iohideventsystem`, entitlement
  `com.apple.iohideventsystem.server`):** `_io_hideventsystem_create_virtual_service` accepts an Admin-type connection or one whose process has
  `com.apple.private.hid.client.event-dispatch` (entitlement bit 3, read from the audit token; the client type is irrelevant).
  backboardd has event-dispatch, event-filter, event-monitor (no `client.admin`, no `service-protected`: other clients cannot *match* protected services, not our concern).
  `hidd` still exists in `/usr/libexec` but is not the server for this.
* **Rotation:** backboardd still takes the orientation from `CMDeviceOrientationManager` ("Updating device orientation from CoreMotion to: %@",
  selectors `startDeviceOrientationUpdatesToQueue:withHandler:` and `deviceOrientationBlocking` are referenced in backboardd; no
  `stop...` or `isDeviceOrientationAvailable` reference found in its strings). iOS 18 CoreMotion additionally has
  `startDeviceOrientationUpdatesPrivateToQueue:withHandler:` and `stopDeviceOrientationUpdatesPrivate`, which the bridge replaces too.
  `CMDeviceOrientation` has `initWithOrientation:andTimestamp:`. backboardd also has `BKAccelerometerInterface` /
  `handleAccelerometerEventWithTimestamp:x:y:z:samplingInterval:` and `hidSystem:receivedUpdatedDeviceOrientation:`
  (an HID-delivered orientation path); not traced.
* **Entry point:** the S20 bridge was two functions called from the GPU inject (`inject.c`); here it is a standalone constructor
  dylib with its own thread (no dependency on Crimson). The S20's `redwood_brightness_guard` and `redwood_software_tone_update` hooks (True Tone
  guard) are dropped. `send` was renamed (the SDK's `sys/socket.h` declares it).

## Not verified

* Everything at run time: the kernel patch compiles and applies (see the commit message) but `/dev/bergamota-sensors` was not booted; the bridge was only compiled/linked/signed.
* That `IOHIDEventSystemClientCreate` from inside backboardd's constructor thread gets a working connection to backboardd's own server
  (the retry loop covers "not up yet"), and that creating a client in the server's own process does not deadlock.
* App-side consumers: CoreMotion apps (`CMMotionManager`) go through `locationd` (`CLWMAccelerometer`, `imu800`, `CLBatchedAccelerometerService`,
  AOP fast-path entitlements in backboardd); whether any of that reads our vendor-page services is unknown, so apps may still see no sensors. Only
  backboardd's consumers (orientation, proximity, CoreBrightness ALS, BKAccelerometer) are in scope here.
* The proximity usage/Placement and the vendor-page usages 0x03/0x04/0x08/0x09/0x0a themselves were not re-derived from the iOS 18 consumers (they are the
  16.5/S20 values; the event types and the ALS requirements above were).
* Gyro and magnetometer units (`sens_gyro_k`, `sens_mag_k`) are the S20's; the Poco F1 has no sensors wired in the kernel at all yet.
* Auto-brightness: the policy's 3-420 nit range is the S20 OLED's; `AppleARMBacklight`/`IODisplayParameters` and `DisplayNitsKey` on this panel/kernel
  are unchecked, so the thread is opt-in (`sensorbridge.autobright`). The ALS service alone (CoreBrightness's own controller) does not need it.
* Whether backboardd keeps the pre-existing crash-loop behaviour with a bridge that fails: a missing `/usr/local/lib/bergamota-sensorbridge.dylib` kills backboardd at launch.
