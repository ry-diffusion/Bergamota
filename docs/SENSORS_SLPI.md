# Sensors on the Poco F1: booting the SLPI and talking to Qualcomm SEE from xnu

Status: research / design (2026-11-28). Nothing here has been run on the phone. No code was written.
Companion: `docs/SENSORS.md` (the kernel `/dev` sample queue and the backboardd bridge, patch 1136). This document is
the producer side: how samples get out of the Snapdragon 845 sensor DSP and into that queue.

Evidence tags used throughout:

* **[O]** observed in this repo, in a phone log, or in a file that is already extracted here.
* **[S]** read in upstream source or data during this session (Linux, libssc, hexagonrpc, sdm845-mainline firmware repo).
  Linux line numbers refer to torvalds/linux `master` as fetched on 2026-11-27 (they move; the symbol names do not).
* **[G]** guess or inference. Each [G] says how a first boot settles it.

---------------------------------------------------------------------------------------------------------------------

## 0. Summary of the key facts

| # | Fact | Tag | Confidence |
|---|------|-----|-----------|
| 1 | Every Poco F1 sensor (accel, gyro, mag, proximity, ambient light, plus a grip/SAR sensor) is owned by the SLPI (Sensor Low Power Island, a Hexagon DSP with its own firmware). The AP cannot read them over its own I2C/SPI. The only way is the SLPI's SEE (Snapdragon Sensor Core, "SSC") over QRTR. | [S] | high |
| 2 | SLPI boots through **TrustZone PAS, pas id 12**, not through an MBA like the modem. The GPU zap shader already uses the same PAS calls with id 13 (`bergamota_a630_pas.c`), so the SCM plumbing exists. | [S][O] | high |
| 3 | Carve-out on beryllium: **0x96b00000, 0x1400000 (20 MiB)**. Not the generic sdm845 address 0x96700000: the beryllium dtsi deletes and redefines it. | [S][O] | high |
| 4 | SMP2P: inbound item **481**, outbound item **430** (modem is 435/428), remote pid 3, doorbell **apss_shared bit 26**. GLINK edge: remote pid 3, doorbell **bit 24**, same SMEM items 478/479/480 as the modem but inside the **apps<->slpi partition (hosts 0<->3)**. That partition exists on this phone: offset 0x15f000, size 0x30000. | [S][O] | high |
| 5 | Power: RPMh ARC **lcx.lvl and lmx.lvl** at the top level (not cx/mx/mss), AOSS QMP `{class: image, res: load_state, name: slpi, val: on}` before start, XO clock. Crash reason is SMEM item **424** (not 421). | [S] | high for the values, medium for "nothing else is needed" |
| 6 | Firmware: `slpi.mdt` + `slpi.bNN` (+ `slpir.jsn`, `slpius.jsn`) live in `image/` of the **`modem` FAT16 partition**, the same place `mba.mbn`/`modem.mdt`/`wlanmdsp.mbn` are read from today. They are NOT extracted anywhere on this Mac (searched all `ios-beryllium*` trees). | [S][O] | high for the location (sdm845-mainline and blob-wrangler both take them from the modem partition's `image/`), medium that they are intact on this unit (not yet listed) |
| 7 | **FastRPC IS required.** Without a FastRPC client on the AP the SLPI's sensor user-PD cannot read its `sensors/config/*.json`, its `registry` files and `sns_reg.conf`; libssc literally polls for the `registry` sensor and says "is hexagonrpcd running?". That is the big extra piece: a GLINK channel `fastrpcglink-apps-dsp`, an invoke transport with hyp-assigned shared memory, `INIT_ATTACH_SNS`, the `adsp_listener` reverse tunnel and ~11 `apps_std` file methods. | [S] | high that it is required, medium on exactly which stage waits for it (G) |
| 8 | SEE wire protocol: QRTR service **0x190** (400), QMI msg **0x0020** Control (request), **0x0021/0x0022** Report small/large (indications); the real payload is a protobuf in TLV 0x01 (u16 length prefix) and TLV 0x10 = report type (1 = large). Full field numbers in section 4, with byte examples I generated and checked against the 67-byte capture published in libssc's docs. | [S] | high |
| 9 | Sensors need files outside the firmware blob: `sensors/config/*.json` (from Android `/vendor/etc/sensors/config`), `sensors/registry/*` (from `/persist/sensors/registry`, holds per-device factory calibration), `sns_reg.conf`, `socinfo/*`, `dsp/sdsp/*`. They are in Android's `vendor` (inside `super`), `dsp` and `persist` partitions, which this kernel cannot read (ext4/erofs, and `persist` is private data). Pull them once from Android/OrangeFox before Android is wiped; keep out of git. | [S] | high |
| 10 | Big engineering items, in order: (a) generalise the single-instance GLINK/QRTR engine in `beryllium_wlan.c` to two edges, (b) PAS bring-up for id 12, (c) in-kernel FastRPC client + file server, (d) protobuf codec + SEE client. | [G] | medium on effort |

Not verified at all and explicitly open: SLPI QRTR node id, instance/version of service 0x190, whether the QMI
service 0x190 appears before or only after the FastRPC listener is serving files, whether `slpi.mdt` is relocatable,
the proximity message flavour (standard vs the Xiaomi "Davinci" struct), the SLPI timestamp base. Section 9 lists how
the first boots answer each.

---------------------------------------------------------------------------------------------------------------------

## 1. SDM845 SLPI remoteproc parameters

### 1.1 Table

| Item | Value | Source |
|------|-------|--------|
| compatible | `qcom,sdm845-slpi-pas` | sdm845.dtsi:3475-3476 [S] |
| Register window (QDSP6SS, TZ-owned, **do not map or read from xnu**) | 0x05c00000, 0x4000 | sdm845.dtsi:3477 [S] |
| PAS id | **12** | qcom_q6v5_pas.c:1515-1520 (`sdm845_slpi_resource_init`) [S] |
| Firmware name (Linux) | `slpi.mdt`; beryllium: `qcom/sdm845/Xiaomi/beryllium/slpi.mbn` (a merged mdt+bNN) | qcom_q6v5_pas.c:1517; sdm845-xiaomi-beryllium-common.dtsi:465-469 [S] |
| Memory region | **0x96b00000, size 0x01400000** (beryllium); generic sdm845.dtsi has 0x96700000/0x1400000 and is deleted by the board file | beryllium-common.dtsi:27 (`/delete-node/ &slpi_mem`), :115-118; sdm845.dtsi:878-881 [S]; docs/DISPLAY_HW.md:116 lists the same range [O] |
| Neighbours | MBA 0x96900000+2 MiB directly below; `pil_spss` 0x97f00000+1 MiB directly above | beryllium-common.dtsi:110-123 [S] |
| Crash reason | SMEM item **424** (global partition) | qcom_q6v5_pas.c:1516 [S] |
| `auto_boot` | true (Linux boots it at probe) | qcom_q6v5_pas.c:1519 [S] |
| Proxy power domains | RPMh **lcx** and **lmx** (cmd-db `lcx.lvl`, `lmx.lvl`), voted to INT_MAX = top level, released on handover | qcom_q6v5_pas.c:1521-1524, :185-211 (`qcom_pas_pds_enable`); rpmhpd.c:128-136, :394-395; sdm845.dtsi:3487-3492 [S] |
| Clock | `xo` = RPMH_CXO_CLK | sdm845.dtsi:3485-3486 [S] |
| AOSS QMP load state | name `slpi` | qcom_q6v5_pas.c:1525; message string in qcom_q6v5.c:24-37 [S] |
| SMP2P | `qcom,smem = <481>, <430>` = [inbound, outbound]; `qcom,local-pid = 0`, `qcom,remote-pid = 3`; GIC SPI 172; doorbell `apss_shared` channel **26** | sdm845.dtsi:1171-1190 [S] |
| SMP2P entries | out `master-kernel` (bit 0 = **stop**); in `slave-kernel` bits: 0 fatal, 1 ready, 2 handover, 3 stop-ack | sdm845.dtsi:1179-1189, :3478-3484 (`interrupt-names = "wdog","fatal","ready","handover","stop-ack"`, wdog = GIC SPI 494) [S] |
| GLINK edge | label `dsps`, `qcom,remote-pid = <3>`, GIC SPI 170, doorbell `apss_shared` channel **24** | sdm845.dtsi:3503-3508 [S] |
| Channels on the edge | QRTR `IPCRTR` (implicit, bound by net/qrtr), FastRPC `fastrpcglink-apps-dsp` (label `sdsp`, `qcom,non-secure-domain`) | sdm845.dtsi:3509-3524 [S] |
| Doorbell register | `apss_shared` mailbox at 0x17990000, write `BIT(channel)` to **offset 12** (same register the modem path uses) | sdm845.dtsi:5377-5381; beryllium_wlan.c:963, :1949-1953 [S][O] |
| AOSS QMP mailbox | 0x0c300000, 0x400 bytes | sdm845.dtsi:5148-5152; `MSS_QMP_PA` beryllium_wlan.c:962 [S][O] |
| SSR/sysmon | `sysmon_name = "slpi"`, `ssctl_id = 0x16` (the modem's is 0x12 and the log shows service 0x2b instance 18 [O]) | qcom_q6v5_pas.c:1527-1528 [S] |
| pd-mapper domains | `msm/slpi/root_pd` and `msm/slpi/sensor_pd`, both QMI instance **90**, only the implicit `tms/servreg` | qcom_pd_mapper.c:331-341, :536-537 [S]; already in our kernel table (docs/WIFI_IOS26.md:738-739) [O]; `slpir.jsn`/`slpius.jsn` in the sdm845-mainline beryllium firmware repo say the same (qmi_instance_id 90, one service tms/servreg) [S] |
| SCM VMIDs | HLOS 0x3, **SSC_Q6 0x5**, ADSP_Q6 0x6, MSS_MSA 0xF | include/dt-bindings/firmware/qcom,scm.h:12-21 [S] |

Compare with the modem values already in `beryllium_wlan.c`: SMP2P 435/428, doorbell 14, GLINK doorbell 12, crash
item 421, ARCs cx/mx/mss. Every SLPI number above is a drop-in replacement of one of those constants.

### 1.2 How Linux starts it (what we must reproduce), qcom_q6v5_pas.c `qcom_pas_start` :308-405

1. `qcom_q6v5_prepare`: interconnect vote (none on slpi), **AOSS load_state on**, clear completions, enable the
   ready/handover/stop IRQs (qcom_q6v5.c:80-104).
2. Proxy domains: lcx and lmx performance state INT_MAX + runtime-resume (`qcom_pas_pds_enable`).
3. `clk_prepare_enable(xo)`.
4. `qcom_mdt_pas_load`: SCM PAS `INIT_IMAGE(12, metadata)` (metadata = ELF header + program headers + hash segment, in
   physically contiguous memory), `MEM_SETUP(12, base, size)`, copy every non-hash PT_LOAD segment (`slpi.bNN`) into
   the carve-out (relocated by `mem_reloc` if the image is relocatable), done with the mapping dropped afterwards.
5. `AUTH_AND_RESET(12)`. TZ checks the signature, locks the region against HLOS (XPU) and releases the Hexagon reset.
6. Wait up to **5000 ms** for the SMP2P `ready` bit (`qcom_q6v5_wait_for_start`).
7. Later the `handover` bit makes Linux drop the proxy votes (`qcom_pas_handover` :419). Stop path: set bit 0 of
   `master-kernel`, wait up to 5 s for `stop-ack` (qcom_q6v5.c:230-250), `PAS SHUTDOWN(12)`, load_state off.

SMP2P outbound/inbound mechanics are exactly the ones already implemented for the modem
(`ms_smp2p_out_setup` / `ms_smp2p_in` / `ms_smp2p_service`, beryllium_wlan.c:1976-2060 [O]).

### 1.3 What the kernel already has and how it maps

| Need | Existing piece [O] | Change |
|------|--------------------|--------|
| SCM PAS calls (fn id = `SCM_64 | SCM_SIP | 0x0200 | cmd`, cmd 1 INIT_IMAGE with arginfo 0x82, 2 MEM_SETUP arginfo 3, 5 AUTH_AND_RESET, 6 SHUTDOWN, SCM_INFO IS_CALL_AVAILABLE probe) | `scm_invoke`, `zap_call`, `zap_availability` in `bergamota_a630_pas.c:60-141` (works, TZ authenticated the zap with id 13) | parameterise the pas id (`SCM_PAS_GPU` is hard-wired in `zap_call`), add IS_SUPPORTED (cmd 7) as a read-only first probe |
| MDT parse, carve-out copy | `bergamota_a630_zap_load` :177-290 (metadata in contiguous WC memory, MEM_SETUP, `ml_io_map` region, per-segment copy) and the MPSS equivalents `ms_mdt_prepare` :1563, `ms_mpss_segments` :1684 which read from the `modem` FAT16 | copy the FAT variant; read `image/slpi.mdt`, `image/slpi.bNN` |
| FAT16 reader for the `modem` partition (8.3 names) | `fat_mount/fat_open/fat_read_chain` beryllium_wlan.c:118-290; `wl_host_read` | none; `slpi.mdt`, `slpi.b00`...`b99`, `slpir.jsn` are 8.3 |
| QMP open + message | `ms_qmp_open` :1264, `ms_qmp_send` :1305 (already sends `load_state ... name: modem`) | send `name: slpi` |
| ARC votes | `bergamota_rpmh_arc_vote` bergamota_rpmh.c:525; allow-list is `cx.lvl, mx.lvl, mss.lvl` | add `lcx.lvl`, `lmx.lvl` to the allow-list; unlike cx/mx an off vote for these two is acceptable (only the SLPI uses them) [G] |
| SMEM partition lookup, item walk, alloc under TCSR mutex 3 | `ms_smem_part(a,b,...)` :1792, `ms_smem_item(a,b,...)` :1818 are generic; `ms_smem_alloc` :1873 is **hard-wired to partition (0,1)** | add host arguments |
| SMP2P item alloc/entries/kick | `ms_smp2p_out_setup` :1976 (hard-wired item 428, pid 0->1, entries master-kernel + ipa, doorbell 14) | parameterise; SLPI has only `master-kernel` |
| GLINK-over-SMEM + QRTR engine | the anonymous `static struct {...} ipc;` (beryllium_wlan.c:2192-2237), `ms_ipc_setup` :5594, `ipc_qrtr_rx` :5493, `ipc_qrtr_announce` :2710; constants :2160-2167 | **single instance, modem-only**: needs an edge context (section 6.2) |
| pd-mapper | `pdm_rx` :3494 with an sdm845 table that already lists the SLPI domains [O] | serve it on the SLPI edge too |
| hyp-assign | `ms_assign_list` :1154, `ms_assign` :1207 | use it for the FastRPC heap |

---------------------------------------------------------------------------------------------------------------------

## 2. Firmware: where it is, and what is (not) on this Mac

* Searched `ios-beryllium`, `-ios18`, `-ios26`, `-bt18`, `-metal`, `research/`, `logs/`, `out/`, plus `~`-level names:
  **no `slpi*`, `NON-HLOS*`, `modem*`, `dsp*` file exists on this Mac.** The partition backups (`backup/*.img`,
  79 partitions) that NOTES.md mentions were made on the Linux machine (`/run/media/dipa/...`) and are not here [O].
  `research/gpu-fw/` only has the GPU files (`a630_gmu.bin`, `a630_sqe.fw`, `zap/`).
* How the kernel finds modem firmware today [O]: it never uses the iOS root. `bergamotawlan=3` and the stage-4 code
  read the phone's own `modem` partition (FAT16, `image/mba.mbn`, `image/modem.mdt`, `image/modem.bNN`,
  `image/wlanmdsp.mbn`, `bdwlan.*`) with `fat_open`, and TFTP serves `/readonly/firmware/image/*` from it
  (docs/WIFI_IOS26.md:232, :363-364, :674-676). The BT firmware is the exception (copied to
  `/usr/standalone/firmware/bergamota/`). The GPU zap came from the sdm845-mainline package and travels in an ADT
  `/bergamota-gpu` node.
* Where SLPI firmware lives [S]: blob-wrangler's beryllium config takes `slpi.mdt`, `slpir.jsn`, `slpius.jsn` from
  `partition = "modem", origin = "image"` (the same list as `adsp.mdt`, `cdsp.mdt`, `modem.mdt`, `mba.mbn`,
  `venus.mdt`). The sdm845-mainline firmware-xiaomi-beryllium package ships the merged `slpi.mbn`
  (7,868,572 bytes, fits the 20 MiB carve-out). So the `.mdt` + `.bNN` parts should already be on the phone in the
  FAT16 `modem` partition, next to the files we read now. **Unverified on this unit**: the boot 30 inventory only
  counted `mba`, `modem.*`, `wlanmdsp`, `qdsp6m.qdb`, `bdwlan.*`, `bdf_*` (docs/WIFI_IOS26.md:232).
* The `.jsn` files are the userspace pd-mapper descriptions; our in-kernel pd-mapper table already encodes the same
  content (instance 90) so the kernel does not need to read them.
* Policy: Qualcomm/Xiaomi firmware stays out of git. This session downloaded no firmware blob, only small text
  configs for reading.

First action for the phone side (read-only, extends the existing `bergamotawlan=3` style inventory): list
`image/slpi*`, print `slpi.mdt` program headers, compare the loadable span with 0x96b00000..0x97f00000 and say whether
it is relocatable (same code as `mdt_summarise`, beryllium_wlan.c:340-420).

---------------------------------------------------------------------------------------------------------------------

## 3. Poco F1 sensor parts and the SLPI

All of these are described by the SEE config JSON shipped for beryllium (sdm845-mainline firmware repo,
`usr/share/qcom/sdm845/Xiaomi/beryllium/sensors/config/*.json` and `sensors_list.txt`) [S]:

| Function | Part (SEE driver) | Bus as the SLPI sees it | IRQ (SSC "chip pin", TLMM number) | Notes |
|----------|-------------------|-------------------------|------------------------------------|-------|
| Accelerometer + gyroscope + temperature + motion detect | **Bosch BMI160** (`sdm845_bmi160_0.json`, `bmi160_0.json`) | `bus_type 1`, `bus_instance 2`, max 3300 kHz; `bus_type 1` is probably SPI [G] | 117 | rails `/pmic/client/sensor_vdd` + `sensor_vddio`; orientation +x +y +z; per-device factory calibration in registry `bmi160_0_platform.accel.fac_cal.*` and `.gyro.fac_cal.*` |
| Magnetometer | **AKM AK0991x** (`sdm845_ak991x_0.json`) | I2C (`bus_type 0`), instance 1, slave 12 (0x0c), 400 kHz | 119 | orient (+y, -x, +z) in the config; factory corr matrix in registry |
| Proximity + ambient light | **AMS TMD2725** (`sdm845_tmd2725.json`) | I2C instance 1, slave 57 (0x39), 400 kHz | 120 | prox near/far thresholds 80/50 in config, ALS coefficients |
| Grip / capacitive ("custom, proximity") | **Analog Devices ADUX1050** (`sdm845_adux1050_0.json`) | I2C instance 1, slave 44 (0x2c) | 120 | not needed for iOS |

Virtual sensors the SLPI derives and lists in `sensors_list.txt`: gravity, game_rv, geomag_rv, ori, akm_rotv, gyro_cal,
mag_cal, pedometer, pickup, tilt, sig_motion, device_orient, resampler, `NonUi`, `Aod`, `psmd`, `har`. Data types we
need are the physical ones: `accel`, `gyro`, `mag`, `proximity`, `ambient_light` (libssc strings, section 4).

Do they hang off the SLPI? **Yes, all of them** [S][high]. libssc's README: direct access is blocked by the
hypervisor; the SLPI owns the QUP buses and the GPIO interrupts, and drives the sensor rails itself through its PMIC
client (`/pmic/client/sensor_vdd[io]`). The mainline beryllium dtsi has no node for any of these parts and the
sensors work on mainline through SSC alone (pmOS `hexagonrpcd-sdsp` + libssc + iio-sensor-proxy, whose upstream tree
contains `drv-ssc-{accel,compass,light,proximity}.c`) [S]. Consequences for xnu:

* Do not configure TLMM pins 117/119/120 or any SSC QUP from the AP; the SLPI does it [G].
* The SLPI votes its rails in its own RPMh DRV; the earlier BT experience (an apps vote-off of the shared 1.8 V ldoa7
  panicked boot 0116hw, docs in NOTES) is not a risk from SLPI votes, but never vote sensor rails from the AP [G].
* Which of the four parts this unit really carries is unproven; the SEE SUID lookup tells us (section 7, stage 8).

---------------------------------------------------------------------------------------------------------------------

## 4. The SEE protocol, wire level

Sources: libssc (codeberg.org/DylanVanAssche/libssc, `data/*.proto`, `src/libssc-client*.c`, `src/libssc-sensor*.c`,
`docs/Qualcomm-Sensorcore-Docs.md`, mock `mocking/ssc_server/ssc.py`), libqmi `data/qmi-service-ssc.json` and
`QMI_SERVICE_SSC` in `qmi-enums.h` [S]. iio-sensor-proxy's ssc drivers are thin wrappers over libssc, so libssc is the
reference [S]. The `.proto` files are reverse-engineered by that project (proto2), not Qualcomm's.

### 4.1 Transport

* **QRTR service id 0x190 (400)** (`QMI_SERVICE_SSC = 0x190`, libssc mock `SSC_SERVICE_ID = 0x190`) [S]. libssc picks the
  node that advertises this service and ignores instance and version, so we should too; the real instance/version
  values are unknown [G]. It appears in the SLPI's QRTR `NEW_SERVER` after it is up.
* QMI over QRTR: plain QMI header (no QMUX): `u8 type` (0 request, 2 response, 4 indication), `u16 txn`, `u16 msg_id`,
  `u16 msg_len`, then TLVs `u8 type, u16 len, value`. All little endian. libssc's own capture matches (type 04, txn
  05 00, id 22 00, len 53 00) [S].
* Messages (libqmi `qmi-service-ssc.json`) [S]:

| Direction | msg_id | Name | TLVs |
|-----------|--------|------|------|
| AP -> SLPI request | **0x0020** | Control | `0x01` Data = `u16 n` + n bytes of protobuf; `0x10` Report Type `u8` (0 small, **1 large**; libssc always sends 1) |
| SLPI -> AP response | 0x0020 | Control | `0x02` result {u16 result, u16 error}; `0x10` client id `u64`; `0x11` response `u32` |
| SLPI -> AP indication | **0x0021** | Report Small | `0x01` client id `u64`; `0x02` Data = `u16 n` + protobuf |
| SLPI -> AP indication | **0x0022** | Report Large | same layout |

  All sensor events (and SUID/attribute/enable replies) arrive as indications, not in the Control response.
  Accept both 0x21 and 0x22.

### 4.2 Protobuf messages (proto2, libssc `data/`)

Wire-type reminders: tag = `(field << 3) | wt`; wt 0 varint, 1 fixed64 (8 B LE), 2 length-delimited, 5 fixed32 (4 B LE).
Floats are fixed32. Repeated scalars are **not packed** (one tag per element), proven by libssc's capture and protobuf-c
defaults [S].

```
SscUid               { fixed64 low = 1; fixed64 high = 2; }                       tags 09, 11
SscClientConfig      { int32 processor = 1 [APPS = 1]; int32 suspend_mode = 2 [WAKEUP = 0]; }   tags 08, 10
SscClientRequestBody { bytes unknown = 1; bytes msg = 2; bool is_passive = 3; }    tags 0a, 12, 18
SscClientRequest     { SscUid uid = 1; fixed32 msg_id = 2; SscClientConfig config = 3;
                       SscClientRequestBody request = 4; }                         tags 0a, 15, 1a, 22
SscClientResponseBody{ fixed32 msg_id = 1; fixed64 timestamp = 2; bytes msg = 3; } tags 0d, 11, 1a
SscClientResponse    { SscUid uid = 1; repeated SscClientResponseBody response = 2; }  tags 0a, 12
```

libssc sends `config = { processor = 1, suspend_mode = 0 }` on every request and never sets `unknown` or `is_passive`
(libssc-client.c:178-205) [S]. The timestamp is the DSP timer, 19.2 MHz (`ssc-common.proto` comment) [S]; whether it
shares the AP's system counter is a guess (section 9).

Sensor-specific bodies (the `msg` bytes):

| Purpose | msg_id (fixed32 in the envelope) | Body message | Notes |
|---------|----------------------------------|--------------|-------|
| SUID lookup, to the fixed SUID sensor `uid = {low = high = 0xABABABABABABABAB}` | request **512** (0x200); reply id **768** (0x300) | req `SscSuidRequest { string data_type = 1; bool enable_updates = 2; bool only_default_values = 3; }` (tags 0a, 10, 18); reply `SscSuidResponse { string data_type = 1; repeated SscUid uid = 2; }` | libssc sends `enable_updates = false`, `only_default_values = true`. Data types: `accel`, `gyro`, `mag`, `proximity`, `ambient_light`, `registry` (service-up probe), also `rotv`. No uid in the reply = no such sensor. |
| Get attributes, to the sensor's own uid | request **1**; reply **128** | req `SscAttrRequest { bool enable_updates = 2; }` (tag 10); reply `SscAttrResponse { repeated SscAttr attr = 1; }`, `SscAttr { int32 id = 1; SscAttrArrayValue value_array = 2; }`, `SscAttrArrayValue { repeated SscAttrValue v = 1; }`, `SscAttrValue { SscAttrValueArray a = 1; string s = 2; float f = 3; fixed64 i = 4; bool b = 5; }`, `SscAttrValueArray { repeated SscAttrValue element = 1; }` | Strings carry a trailing NUL ("lsm6dsm\0" in the doc sample). Attribute ids (libssc-common-private.h): 0 name, 1 vendor, 2 type, 3 available, 4 version, 5 api, **6 sample rates**, 7 resolutions, 8 fifo size, 11 ranges, 12 operating modes, 13 dri, **16 stream type** (0 continuous, 1 on-change), 17 dynamic, 18 hardware id, **20 mount matrix** (3x4 floats; libssc uses the first 9), 21 physical sensor. |
| Enable, continuous (accel, gyro, mag) | request **513** (0x201) | `SscEnableConfigRequest { float sample_rate = 1; }` (tag 0d + 4 B) | reply id 768 `SscConfigResponse { float sample_rate = 1; string mode = 5; }`, then measurements |
| Enable, on-change (proximity, ambient_light) | request **514** (0x202) | empty body (omit `msg`) | |
| Disable | request **10** | none | |
| Measurement | event **1025** (0x401); proximity may use **769** (0x301) | accel `{ repeated float acceleration = 1 (m/s^2, x y z); int32 accuracy = 2; }`; gyro `{ repeated float velocity = 1; int32 accuracy = 2; }`; mag `{ repeated float magnetic_field = 1 (uT); int32 accuracy = 2; }`; light `{ repeated float intensity = 1 ([0] lux, [1] raw); int32 accuracy = 2; }`; proximity `{ int32 near = 1; int32 distance = 2; int32 accuracy = 3; }` | accuracy 0..3. The Xiaomi davinci variant sends proximity as `{ repeated bytes data = 1; int32 accuracy = 2; }` under 1025 with a C struct; which form the TMD2725 on beryllium uses is unknown, decode both (field 1 wire type varint vs length-delimited) [G]. |

All units are the SLPI's: accel m/s^2, gyro rad/s, mag uT. The iOS conversion (g, sign of z on a flat device) belongs to
the bridge in `docs/SENSORS.md`; Android-style readings have z = +9.81 flat and face up, CoreMotion wants z = -1 g
there [G].

### 4.3 Byte examples (generated with a throwaway encoder, not libssc; inputs/outputs checked by hand)

Notation: hex bytes. The encoder reproduces the 67-byte `SscClientResponse` capture in libssc's docs when fed its
fields (length 67 matches the doc's "Protobuf data is 67 bytes").

**SUID lookup for `accel`** (QMI txn 1). Protobuf, 46 bytes:

```
0a 12 09 ab ab ab ab ab ab ab ab 11 ab ab ab ab ab ab ab ab     uid {low, high} = SUID sensor
15 00 02 00 00                                                  msg_id = 512 (fixed32)
1a 04 08 01 10 00                                               config {processor 1, suspend 0}
22 0d 12 0b 0a 05 61 63 63 65 6c 10 00 18 01                    request.msg = {"accel", enable_updates 0, default_only 1}
```

Full QMI frame (62 bytes), what goes into the QRTR payload to the node/port that announced service 0x190:

```
00 01 00 20 00 37 00                         request, txn 1, msg 0x0020, length 55
01 30 00  2e 00  <the 46 protobuf bytes above>   TLV 0x01: length 0x30 = 2 + 46, u16 protobuf length 0x2e
10 01 00  01                                 TLV 0x10: report type = large
```

**SUID lookup for `registry`** (service-up probe): same, last body becomes
`22 10 12 0e 0a 08 72 65 67 69 73 74 72 79 10 00 18 01`.

**Enable accel at 50 Hz**, with a placeholder sensor uid `low = 0x1122334455667788`, `high = 0x99aabbccddeeff00`
(use the uid from the lookup reply), QMI txn 2. Body `0d 00 00 48 42` (float 50.0 = 0x42480000):

```
0a 12 09 88 77 66 55 44 33 22 11 11 00 ff ee dd cc bb aa 99
15 01 02 00 00                               msg_id = 513 (continuous enable)
1a 04 08 01 10 00
22 07 12 05 0d 00 00 48 42
```

QMI frame: `00 02 00 20 00 31 00  01 2a 00 28 00 <40 protobuf bytes>  10 01 00 01`.

**Disable**: `... 15 0a 00 00 00 1a 04 08 01 10 00 22 00`.

**A reply to decode** (from libssc's docs, SUID reply for `gravity`; use it as the decoder unit test; it is the 67-byte
protobuf, the QMI part is `04 05 00 22 00 53 00 01 08 00 <8 B client id> 02 45 00 43 00`):

```
0a 12 09 ab*8 11 ab*8                      uid = SUID sensor
12 2d                                      response (length 45)
   0d 00 03 00 00                          msg_id = 0x300 = 768
   11 8a a3 6a 89 3c 35 00 00              timestamp
   1a 1d                                   msg (29 bytes): SscSuidResponse
      0a 07 67 72 61 76 69 74 79           data_type "gravity"
      12 12 09 <8 B low> 11 <8 B high>     one uid
```

**An accel measurement as the kernel will see it** (made-up values 0.1, 9.81, -0.2, accuracy 3):

```
0a 12 09 88 77 66 55 44 33 22 11 11 00 ff ee dd cc bb aa 99
12 21  0d 01 04 00 00                      msg_id 1025
       11 a2 9c d6 2c 4a 00 00 00          timestamp (ticks, 19.2 MHz)
       1a 11  0d cd cc cc 3d  0d c3 f5 1c 41  0d cd cc 4c be  10 03
```

### 4.4 Codec scope for the kernel

No protobuf library is needed. Required pieces, all small: varint read/write, a skip routine for unknown fields (by wire
type), encoders for exactly the six request messages above, decoders for `SscClientResponse` (walk `response`
entries), `SscSuidResponse`, the attribute tree (only ids 0, 3, 6, 16, 20 matter), `SscConfigResponse`, and the five
measurement bodies. Test it on the Mac first with the vectors above (the kernel repo already uses Mac harnesses for
WPA and the 802.11 frames, docs/WIFI_IOS26.md 6.3 [O]).

---------------------------------------------------------------------------------------------------------------------

## 5. Startup order and what must be up first

### 5.1 Order (Linux/pmOS reality [S], mapped to xnu)

1. AP side power/votes: AOSS load_state `slpi` on, lcx/lmx at top level, XO (section 1.2).
2. PAS load + auth + reset, wait `ready` (SMP2P). The SLPI runs its root image, brings up GLINK and QRTR.
3. SLPI announces itself on QRTR (HELLO, then NEW_SERVER for its services; expect at least the SSCTL service 0x2b with
   instance 0x16 and the servreg notifier 0x42 with instance 90, by analogy with the modem's 0x2b/18 and 0x42/180 in
   the boot log [O/G]). It asks the **pd-mapper** (QRTR service 0x40, we already implement it) for `msm/slpi/root_pd`
   and `msm/slpi/sensor_pd`.
4. The SLPI starts its **sensor user PD** (`sensor_pd`). Everything SEE needs (`sns_reg_config`, registry,
   `sensors/config/*.json`, socinfo) is read **by the SLPI from the AP's filesystem** through FastRPC reverse calls
   (details in 5.3). The Android daemon is `sscrpcd sensorspd` (libssc docs); the mainline one is
   `hexagonrpcd -f /dev/fastrpc-sdsp -d sdsp -s` (`data/hexagonrpcd-sdsp.service.in`) [S].
5. Only then does the `registry` data type exist; libssc waits for it (`wait_for_sensor_service` polls
   `SUID("registry")` once per second, up to 100 tries, and warns "is hexagonrpcd running?") and only then looks up
   `accel`, `gyro`, ... [S].

### 5.2 Is FastRPC required? **Yes.**

Evidence: libssc docs ("sscrpcd reads various JSON configs from the persist partition which the SSC needs to control
the sensors such as bus type, bus address"), the hexagonrpc README ("FastRPC is used ... to serve files to the remote
processors"; the HexagonFS table maps `sensors/config`, `sensors/registry`, `sns_reg.conf`, `socinfo`, `dsp`), the
libssc warning text, and the sensor configs themselves: bus type, instance, slave address, IRQ pin, rail names are all
in files that only the AP can serve [S]. There is no AP-side QMI/QRTR alternative: those values are not sent over
SEE, they are pulled by the DSP.

What exactly blocks without it is unknown [G]: either the whole sensor PD (so service 0x190 never appears) or only the
registry sensor (0x190 appears, SUID("registry") stays empty). Stage 6 of the plan measures this before any FastRPC
code exists: bring the SLPI up and watch NEW_SERVER.

This is the biggest extra piece (section 6.3): roughly an invoke transport (~300 lines), listener loop and `localctl`
(~250), `apps_std` + `apps_mem` (~500), file table (~150). About 1.2-1.5 kLOC of C, all polled, similar to the
existing TFTP server in size.

### 5.3 The FastRPC protocol, as much as xnu needs of it (drivers/misc/fastrpc.c [S])

* Channel: GLINK channel `fastrpcglink-apps-dsp` on the SLPI edge (sdm845.dtsi:3509-3512). Device label `sdsp`.
* Request, one GLINK message of **40 bytes** (`struct fastrpc_msg`, fastrpc.c:196-204): `s32 client_id, s32 tid,
  u64 ctx, u32 handle, u32 sc, u64 addr, u64 size`. `ctx` = `(index << 8) | (seq << 16) | pd` with the PD in the low
  bits (ROOT 0, USER 1, **SENSORS 2**, fastrpc.c:126, :50-53, :1274); `addr/size` = physical address and page-rounded
  size of the payload buffer.
* Response, one GLINK message of **16 bytes** (`struct fastrpc_invoke_rsp`, :206-209): `u64 ctx, s32 retval` (+pad).
  A `ctx` of 0xABCDABCD is a PD-exit notification and is dropped (:2738-2744).
* Scalars `sc` (fastrpc.c:99-108): `attr << 29 | method << 24 | n_in << 16 | n_out << 8 | n_in_handles << 4 |
  n_out_handles`.
* Payload buffer layout (fastrpc.c:994-1030 comment, :1082-1190 `fastrpc_get_args`): argument array
  (`{u64 pv, u64 len}` per scalar), then `{u32 num, u32 pgidx}` list, then `{u64 addr, u64 size}` page list, then
  `u64 fdlist[16]`, `u32 crclist[64]`, a `u32` poll word; inline arguments follow at 128-byte alignment. Input buffers
  are copied in, outputs copied out after the response.
* Attach: `FASTRPC_IOCTL_INIT_ATTACH_SNS` = invoke on **handle 1** (`FASTRPC_INIT_HANDLE`), `sc = SCALARS(method 0, 1
  in, 0 out)`, one input word = client id, with `pd = 2` in `ctx` (fastrpc.c:1830-1842, :2291-2292).
* Then hexagonrpcd: `remotectl_open` ("adsp_default_listener") on the DSP's static handle 0, method 0 `open`, then
  `adsp_default_listener_register` (method 0), then loops on `adsp_listener_next2` (method **4**, 2 input words, 1 input
  buffer, 4 output words, 1 output buffer) where each return carries the DSP's reverse call (ctx, handle, scalars,
  in-buffers) and the next call carries our results. Reverse interfaces served: `apps_remotectl` (handle 0: open
  returns the index of the named interface, -5 if unknown; close), **`apps_std`** (handle 1: freopen 1, fflush 2,
  fclose 3, fread 4, fseek 9, fopen_with_env 19, opendir 26, closedir 27, readdir 28, mkdir 29, stat 31), **`apps_mem`**
  (handle 2: `request_map64`, method 2). Method tables with argument layouts: hexagonrpcd/interface/*.c [S].
* The virtual file tree the SLPI sees (hexagonrpcd/rpcd_builder.c [S]):

| SLPI path | Backing (Android source) | Our source |
|-----------|--------------------------|------------|
| `/mnt/vendor/persist/sensors/registry/registry` and `/persist/...` | `/persist/sensors/registry` (calibration, written back by the DSP) | staged files + RAM-backed writes |
| `/vendor/etc/sensors/config/*.json`, `/system/vendor/...` | Android `/vendor/etc/sensors/config` | staged files |
| `/vendor/etc/sensors/sns_reg_config` | `sensors/sns_reg.conf` | staged file |
| `/vendor/etc/acdbdata` | audio, not needed for sensors | skip, answer ENOENT |
| `/sys/devices/soc0/{hw_platform,platform_subtype,platform_subtype_id,platform_version,revision,soc_id}` | sysfs socinfo; pmOS ships them as plain files (beryllium soc_id is `321`) | files, or generate from SMEM socinfo [G] |
| `/usr/lib/qcom/adsp/*` | `dsp/sdsp` (skel libraries, `fastrpc_shell_2` 857 KB, `libchre_slpi_skel.so` 3 MB) | not needed for SEE alone [G]; serve if the DSP asks |

* Memory (Linux): message buffers for the sdsp come from the **`fastrpc_mem` shared-dma-pool**, 16 MiB, 4 MiB aligned,
  below 4 GiB (sdm845.dtsi:894-900), because the sdsp compute-cb has no IOMMU (`sid == 0`, fastrpc.c:1115-1118,
  :525-535). At probe the whole region is **hyp-assigned from HLOS to {HLOS, MSS_MSA, SSC_Q6, ADSP_Q6}**
  (`qcom,vmids`, sdm845.dtsi:3513-3515; fastrpc.c:2508-2547, :2586-2588). We need an equivalent heap: a physically
  contiguous, <4 GiB range outside xnu's managed RAM, assigned with `ms_assign_list`. SSC_Q6 (0x5) is the essential
  VMID; keeping Linux's four is the proven configuration [S], dropping the other two is a guess [G].
  The existing Wi-Fi pool occupies 0xe0100000..0xe0d00000 ("never reuse", docs/WIFI_IOS26.md:1176 [O]); a new range
  must be carved from the ADT memory map the same way. Which address is free is undecided [G].

### 5.4 What else must be up first (xnu side)

* SMEM hwlock/TCSR mutex 3, SMEM partition 0<->3, QMP, RPMh ARC: all already proven for the modem except partition
  0<->3 and `lcx/lmx` [O].
* **pd-mapper must answer on the SLPI edge**; the SLPI asks for its domains early. Our `pdm_rx` handles
  `msm/slpi/root_pd` and `msm/slpi/sensor_pd` already (boot 42 saw 7 requests from the modem) [O].
* Do **not** require rmtfs/TFTP for the SLPI [G]: no evidence it uses them; announce only what it looks up.
* The modem and the SLPI are independent subsystems; the SLPI can be started without the MPSS. Since one boot stage
  currently runs the modem and the Wi-Fi service, run the SLPI as its own gate (`bergamotaslpi`) that works with or
  without `bergamotawlan=5`.

---------------------------------------------------------------------------------------------------------------------

## 6. Kernel design

### 6.1 Components

1. **slpi_rproc**: PAS 12 load/start/stop, QMP, ARC, SMP2P (items 481/430, bit 26), watching ready/fatal/handover/
   stop-ack by polling like the modem path. New code, mostly copies of `ms_*` with different constants; the PAS part
   is the `zap_call` path with pas id 12 and the FAT reader from the modem path.
2. **glink_edge**: the transport. Today `ipc` is one global, modem-only (185 references, ~110 of them to transport
   fields: `desc tx tx_len rx rx_len features version_acked rcid open_acked li ri txq txq_n intent_asked next_liid node
   hello_seen closed bad_rx`). Turn those members into `struct glink_edge { host (1 or 3), doorbell (12 or 24),
   smem items (478-480), ... }` with one instance per remote, leaving rmtfs/TFTP/WLFW state in `ipc`. Channels become
   a small table instead of the single hard-wired lcid `GLINK_LCID 1` (beryllium_wlan.c:2166): the SLPI edge needs
   `IPCRTR` and `fastrpcglink-apps-dsp`.
3. **qrtr node table**: today the modem's node id is "learned from its packets" (`ipc.node`, one value) and replies go
   to that node. With two edges every reply must go to the edge the peer's node came from. Needed: node -> edge map;
   NEW_SERVER from either edge goes into one service table; our own announcements (pd-mapper 0x40, at node 1 port
   0x4002) are sent to each edge after its HELLO. The SLPI's QRTR node id is unknown (libssc's doc capture decodes to
   node 2, but I read that off an strace line and give it low confidence) [G].
4. **see_client**: protobuf codec + QMI framing + a small state machine: wait for service 0x190 -> SUID("registry")
   -> SUID per data type -> attributes -> enable -> events.
5. **fastrpc**: invoke transport, heap, listener, apps_std/apps_mem, file table (section 5.3).
6. **sample sink**: `bergamota_slpi_sample_push(kind, ts_ticks, f32 v[3], accuracy)` into the queue of `docs/SENSORS.md`.
   Timestamp conversion: ns = ticks * 625 / 12 (19.2 MHz).

### 6.2 Where to put the code

`beryllium_wlan.c` is 9.6 kLOC of `static` code and every helper above is `static`. Lowest risk: add the SLPI as new
sections in that file behind a new gate first (so the helpers stay reachable and the Wi-Fi path is unchanged until the
edge refactor lands), then split `bergamota_slpi.c` out later. The edge refactor is mechanical but touches the working
Wi-Fi stack; do it as its own patch and re-test Wi-Fi (join + 17 MB transfer) before stacking SLPI on top.

Threading: the modem path is one polled service thread (`ms_ipc_run`, adaptive 1/20 ms polling). Run the SLPI edge in
the same loop (one extra poll per iteration) rather than a second thread; the SMEM hwlock is hardware-serialised but
`ms_smem_alloc` and the GLINK state are not locked against a second thread.

Persistence of files for the file server (section 5.3): either (a) a launchd feeder that pushes the staged tree into
the kernel over a sysctl/ioctl before the listener starts, or (b) packing the ~100-300 KB tree into an ADT property
like the GPU firmware. Because the DSP's reverse calls simply wait until `adsp_listener_next2` is called, the SLPI can
boot early and the listener can start after the files are present [G].

Alternative if the in-kernel `apps_std` turns out painful: expose only the FastRPC invoke transport to userland and run
a port of `hexagonrpcd` as a launchd job (GPL-3 C, needs the three Linux ioctls INIT_ATTACH_SNS/INVOKE/ALLOC_DMA_BUFF
replaced). It costs a user client and a buffer ABI but moves the file serving out of the kernel.

### 6.3 Boot-args (all off by default)

`bergamotaslpi=N` selects the highest stage to run (stages are cumulative and listed in section 7); further
independent gates: `bergamotaslpiauth=1` (the only stage that releases the DSP from reset), `bergamotaslpifrpc=1`,
`bergamotaslpisee=<mask>` (accel 1, gyro 2, mag 4, prox 8, light 16), `bergamotaslpistop=N` (stop cleanly before step N,
like `bergamotamssstep`). Remember the boot-args field only holds ~607 usable bytes and `mkpayload` truncates silently
(NOTES).

---------------------------------------------------------------------------------------------------------------------

## 7. Bring-up plan, ordered by risk

Every step logs `BERGAMOTA SLPI: step N ...` and calls `bergamota_log_sync()` before it runs (the modem work learned
this: boots 32-35 lost their last lines to buffered logs). Panics stay visible: **never** combine with
`bergamotafb_noconsole=1` (hidden `ml_io_unmap` panic of boots 34/35). One stage per boot, user present for stages 4+,
phone charged above ~3.6 V (NOTES, 2026-11-28).

| Stage | What it does | Log checkpoints that prove it | Risk |
|-------|--------------|-------------------------------|------|
| **1 probe (read-only)** | FAT-walk `modem:image/` for `slpi.mdt`, `slpi.b00..`, `slpir.jsn`, `slpius.jsn`; print program headers; check span vs 0x96b00000+0x1400000, relocatable flag; sizes. SMEM partition 0<->3: free pointers, items present (478/479/480 absent = SLPI cold; 424, 481 absent). cmd-db lookup of `lcx.lvl`/`lmx.lvl` (ARC level maps). `dsp` and `persist` superblock magic (0xEF53 at +0x438) to confirm ext4. | `slpi.mdt N phdrs, loadable span 0x... .. 0x..., fits/does not fit`; `partition 0<->3 free-uncached 0x20`; `lcx.lvl addr 0x... levels N`; `no write`  | very low (same reads as `bergamotawlan=3`) |
| **2 power only** | QMP open + `load_state slpi on`; ARC vote lcx, lmx to top; hold 2 s; vote back to 0, `load_state off`. No SCM, no carve-out access. | `QMP "{class: image, res: load_state, name: slpi, val: on}" consumed`; `ARC lcx.lvl -> level N`; `ARC lmx.lvl -> level N`; same for off | low (QMP and ARC are proven; only the two new resource names) |
| **3 SMP2P + GLINK descriptors, no DSP** | allocate SMP2P item 430 in 0<->3 (entry `master-kernel`, version 1), kick bit 26; allocate GLINK items 478/479 in 0<->3; kick bit 24; read item 481/480 (expect absent). Pure SMEM writes + doorbells into a cold DSP (doorbells to a held-in-reset DSP are ignored [G]). | `smp2p item 430 ready at +0x...`; `glink descriptor/TX FIFO ok`; `inbound 481 absent (SLPI cold)` | low-medium: first writes into partition 0<->3 |
| **4 load, no auth** | IS_SUPPORTED(12); metadata to contiguous WC memory; INIT_IMAGE(12); MEM_SETUP(12, 0x96b00000, size); map carve-out with `ml_io_map_unmappable`, copy segments, **unmap**; SHUTDOWN(12); log hashes of what was copied. DSP is never released. | `SCM PAS INIT_IMAGE completed status=0 result=0`; `MEM_SETUP ... 0`; `segment k (slpi.bNN): N bytes -> +0x...`; `carve-out unmapped`; `SHUTDOWN 0` | **medium**: first write into the carve-out next to the MBA one that froze boots 32-35 (cause then was a hidden unmap panic, fixed in 1110); first PAS use with id 12 |
| **5 start the DSP** (`bergamotaslpiauth=1`) | stage 2 votes + stage 3 SMP2P + stage 4 load, then AUTH_AND_RESET(12); poll SMP2P 481 every 10 ms up to 5 s: print every entry change; `slave-kernel` bit 1 = ready; bit 0 = fatal (print SMEM 424). Stay up 30 s watching bit 2 (handover) then drop proxy votes. Then stop: set `master-kernel` bit 0, wait bit 3 (stop-ack, 5 s), SHUTDOWN(12), load_state off, ARC off. | `AUTH_AND_RESET ... status=0`; `smp2p 481 negotiated version 1`; `slave-kernel ready after N ms`; `handover`; `stop-ack after N ms`; clean teardown; phone still up | **highest**: releases a signed Hexagon core that starts driving rails, TLMM and buses. Watch for an instant reset (XPU/TZ). Same shape as boot 39 (`MPSS READY`) for the modem |
| **6 QRTR on the SLPI edge** | stage 5 kept running; GLINK VERSION/OPEN `IPCRTR`, QRTR HELLO, collect NEW_SERVER; announce pd-mapper (0x40) to the SLPI; serve GET_DOMAIN_LIST for the two SLPI domains. **No FastRPC yet.** Settles whether service 0x190 needs FastRPC. | `glink: slpi opened IPCRTR`; `qrtr: hello from node N` (learns the node id); `slpi offers service 66 (0x42) instance 90`; `service 43 (0x2b) instance 22`; `pd-mapper request for msm/slpi/...`; **`slpi offers service 400 (0x190)` yes/no** | medium (new edge in an engine that is single-instance today; do the refactor first) |
| **7 SEE probe (no FastRPC)** | If 0x190 appeared: send SUID("registry") then SUID("accel"); print raw hex of every indication and the decode. If it did not: skip to 8. | `see: tx 62 bytes txn 1`; `see: control response result 0`; `see: report large id 0x22 N bytes`; decoded `SUID registry: 0 uids` (expected without files) | low, protocol only (codec first tested on the Mac with section 4.3 vectors) |
| **8 FastRPC** (`bergamotaslpifrpc=1`) | GLINK open `fastrpcglink-apps-dsp`; heap + hyp-assign {HLOS, SSC_Q6 (+ the two proven extras)}; INIT_ATTACH_SNS invoke; remotectl open `adsp_default_listener`; listener loop; log **every reverse call with its path** (this yields the exact file list the SLPI wants); serve staged files; answer missing paths with ENOENT. | `fastrpc: channel open`; `heap 0x... assigned status 0`; `attach sns retval 0`; `listener registered`; `apps_std fopen /mnt/vendor/persist/sensors/registry/registry/... -> ok/ENOENT`; `apps_std fopen /vendor/etc/sensors/config/sdm845_bmi160_0.json`; then `SUID registry: 1 uid` | medium-high effort, medium risk: hyp-assign of a new region (wrong range = SCM error, or worse if it overlaps live xnu memory), first FastRPC |
| **9 sensors** | SUID + attributes for `accel`, `gyro`, `mag`, `proximity`, `ambient_light`; print name/vendor/available/sample rates/stream type/mount matrix; enable accel at 50 Hz (or the nearest advertised rate), log first 10 samples; flat on a table z should be about +/-9.81 m/s^2; rotate the phone; then gyro, mag; proximity/light on-change (cover the sensor). Feed the sample queue. Compare the first event timestamp with `CNTPCT_EL0`. | `see: accel uid 0x.. 0x.. name bmi160 vendor ... rates [..]`; `accel sample 50.0 Hz`; `accel x y z`; `prox near 1 far 0`; `als 123.4 lux`; `ts base: slpi - cntpct = N ticks` | low once 8 works |
| **10 hardening** | stop-on-reboot handshake, SLPI crash handling (fatal -> log SMEM 424, reload), 60 s soak, battery cost of sensors with the screen off (the earlier drain investigation makes this matter), rate throttling when the screen is off, restart of the listener on `PD exit` notifications. | no panic after 1 h; fatal path exercised | low |

### 7.1 Things that can hard-hang or reset the SoC (each behind its own boot-arg)

1. **AUTH_AND_RESET(12)** with bad metadata/segments, or touching the carve-out after it: TZ/XPU protects
   0x96b00000..0x97f00000 against HLOS from that moment; an HLOS access is an XPU violation that resets the SoC [G, by
   analogy with the zap shader and the MBA region]. Rules: copy before AUTH, unmap the carve-out before AUTH (the
   unmappable mapping, never a Normal-cacheable one that can be speculated into the protected range), zero further
   references. Gate `bergamotaslpiauth=1`.
2. The first write into the carve-out (stage 4): the MBA neighbour froze boots 32-35 [O]. The known cause is fixed
   (`ml_io_map_unmappable`, 1110) but the same code path is new for this region. Gate stage 4 behind its own arg.
3. The running SLPI itself: it powers sensor rails and drives QUP/TLMM. If the board has a sensor rail in a state the
   SLPI does not expect (we never voted it, ABL may have), a bad sequence can brown out or the SLPI can fatal; a
   TZ-configured "subsystem crash -> system reset" policy is possible [G]. Keep the hold time short in stage 5,
   user present.
4. Votes: lcx/lmx top-level votes are safe in principle; **never vote cx/mx/sensor rails down** (ldoa7 precedent,
   `bergamota_rpmh_arc_vote` refuses cx/mx off for the same reason) [O].
5. Mapping or reading the QDSP6SS window 0x05c00000 from xnu (gated hardware, the existing code deliberately never
   reads RMB/QDSP6 for the modem): do not map it at all.
6. Hyp-assign of the FastRPC heap: a range that overlaps xnu's RAM or another carve-out. Pick it only from addresses
   verified outside the ADT memory map, size it 4 MiB first (Linux uses 16 MiB), and always assign HLOS back in the dst
   list. Gate `bergamotaslpifrpc=1`.
7. SMEM writes into 0<->3 while the SLPI is running: use `ms_smem_lock` and keep prints out of the locked region (the
   modem code spins on the same hwlock; "nothing in here prints" comment at :1884) [O].
8. Starting stage 6+ without stage 5's clean-stop path working: a leaked running SLPI across a kernel restart. The
   watchdog (`bergamotawdt=30`) restarts the kernel; make teardown mandatory on panic paths [G].
9. Doorbell bits 24/26 fired before the DSP is up are harmless [G]; firing the **wrong** bit (an off-by-two between
   modem 12/14 and slpi 24/26) pokes a different subsystem. Unit-test the constant table against sdm845.dtsi.
10. Keeping the Wi-Fi service on at the same time: two long-lived polled engines racing on `ms.*`, `wl.smem` and the
    hwlock. Until the edge refactor lands, run SLPI stages only with `bergamotawlan=0`.

---------------------------------------------------------------------------------------------------------------------

## 8. Staging the data files (before Android goes away)

From Android or OrangeFox with `adb` (user's approval needed for any pull; keep every pulled file out of git, device
dumps and calibration are private to this unit):

| Pull | Path on Android | Notes |
|------|-----------------|-------|
| sensors config | `/vendor/etc/sensors/config/`, `/vendor/etc/sensors/sns_reg_config` | text JSON; `config_list.txt`, `sensors_list.txt` too |
| sensors registry | `/mnt/vendor/persist/sensors/registry/registry/` (and `/persist/sensors/registry` in recovery) | **this unit's factory calibration** (accel/gyro/mag fac_cal, proximity thresholds) |
| sdsp libs (maybe) | `/vendor/dsp/sdsp/` (the `dsp` partition) | only if stage 8 shows the SLPI asking for them |
| socinfo | `/sys/devices/soc0/{hw_platform,platform_subtype,platform_subtype_id,platform_version,revision,soc_id}` | 6 tiny files |

The sdm845-mainline `firmware-xiaomi-beryllium` package already contains a generic copy of all of this, extracted from a
Poco F1 (config, registry templates, dsp/sdsp, socinfo, `slpi.mbn`). It is a possible shortcut but is somebody else's
unit calibration and a redistributed Qualcomm/Xiaomi binary set; the user decides whether to fetch it. If the registry
is generated instead, hexagonrpc ships `tools/sscregistrygen` which builds `registry/*` from the config JSON files
(usage in its header) [S].

---------------------------------------------------------------------------------------------------------------------

## 9. Open questions and the first boot that answers each

| Question | Current belief | Answered by |
|----------|----------------|-------------|
| `slpi.mdt/.bNN` present and intact on this unit | yes [G] | stage 1 |
| `slpi.mdt` relocatable, loadable span vs the 20 MiB carve-out | fits [G] | stage 1 |
| `lcx.lvl` / `lmx.lvl` exist in cmd-db | yes (Linux sdm845 uses them) [G] | stage 1 and 2 |
| Is the SLPI really cold at kernel entry (ABL did not start it) | cold, since Android's kernel starts it [G] | stage 1 (items 424/481/480 absent) |
| TZ accepts PAS id 12 from this kernel | yes, id 13 worked [G] | stage 4 |
| SLPI reaches `ready` within 5 s | yes (Linux waits 5 s) | stage 5 |
| QRTR node id of the SLPI | unknown, learn from HELLO | stage 6 |
| Service 0x190 appears without FastRPC | unknown | stage 6 vs 7 |
| Instance/version of service 0x190 | unknown, ignore like libssc | stage 6 |
| Exact list of files the SLPI opens | `sensors/config/*`, `registry`, `sns_reg_config`, socinfo [S/G] | stage 8 log |
| Heap VMIDs needed (just SSC_Q6, or Linux's four) | Linux's four are proven | stage 8 |
| Proximity message flavour (standard vs Davinci) | unknown | stage 9 |
| SLPI timestamp base equals the AP system counter | possibly [G] | stage 9 |
| Which of BMI160, AK0991x, TMD2725 are fitted on this unit | all [S config, G unit] | stage 9 |
| Interaction with `bergamotawlan=5` on one poll loop | needs the edge refactor | before stage 6 |

---------------------------------------------------------------------------------------------------------------------

## 10. Source index

Linux (torvalds/linux master, 2026-11-27): `drivers/remoteproc/qcom_q6v5_pas.c` (:41-84 descriptor, :185-211 proxy
domains, :308-405 start, :419 handover, :1515-1528 sdm845 slpi, :2039 compatible), `qcom_q6v5.c` (:24-37 QMP message,
:80-104 prepare, :171-196 ready, :230-250 stop), `arch/arm64/boot/dts/qcom/sdm845.dtsi` (:878-881 slpi_mem, :894-900
fastrpc_mem, :1171-1190 smp2p-slpi, :3475-3525 slpi_pas + glink-edge + fastrpc, :5148 aoss_qmp, :5377 apss_shared),
`sdm845-xiaomi-beryllium-common.dtsi` (:27, :115-118, :465-469), `drivers/pmdomain/qcom/rpmhpd.c` (:128-136, :394-395),
`drivers/soc/qcom/qcom_pd_mapper.c` (:331-341, :530-538), `drivers/misc/fastrpc.c` (:36-134 constants and
scalars, :196-209 wire structs, :994-1190 payload layout, :1255-1281 send, :1830-1842 attach, :2291 ATTACH_SNS,
:2508-2600 reserved mem + vmids, :2719-2760 response callback), `include/dt-bindings/firmware/qcom,scm.h`.

libssc (codeberg.org/DylanVanAssche/libssc, main): `data/ssc-common.proto`, `ssc-sensor-suid.proto`, `-accelerometer`,
`-gyroscope`, `-magnetometer`, `-light`, `-proximity`; `src/libssc-client.c` (:165-243 request build), `-client-private.h`
(SUID uid, msg ids 512/768), `libssc-common-private.h` (ids 513/514/10/1/128/769/1025, attribute ids),
`libssc-sensor.c` (:277-500 reply handling, :566-612 discover, :640-690 registry probe), `libssc-sensor-*.c` (data
type strings), `docs/Qualcomm-Sensorcore-Docs.md`, `mocking/ssc_server/ssc.py`. libqmi `data/qmi-service-ssc.json`,
`qmi-enums-ssc.h`.

hexagonrpc (github.com/linux-msm/hexagonrpc): `README.md`, `hexagonrpcd/rpcd.c`, `rpcd_builder.c`, `localctl.c`,
`interface/{apps_std,apps_mem,adsp_listener}.c`, `data/hexagonrpcd-sdsp.service.in`, `tools/sscregistrygen.c`.

Device data (gitlab.com/sdm845-mainline/firmware-xiaomi-beryllium; only text configs were read, no blob downloaded):
`usr/share/qcom/sdm845/Xiaomi/beryllium/sensors/{sensors_list.txt, config/*.json, sns_reg.conf}`,
`lib/firmware/qcom/sdm845/Xiaomi/beryllium/{slpir.jsn, slpius.jsn}`, tree listing. blob-wrangler
(github.com/samcday/blob-wrangler) `configs/xiaomi,beryllium.toml` for the partition of origin.

This repo: `research/xnu-local/dxb/xnu/bsd/dev/beryllium_wlan.c`, `bergamota_a630_pas.c`, `bergamota_rpmh.c`;
`docs/WIFI_IOS26.md` (:232, :363, :674, :738-739, :1176), `docs/DISPLAY_HW.md` (:98-125), `NOTES.md` (2026-11-28 sensors
entry); the SMEM partition table with `0<->3` at 0x15f000/0x30000 is from `../ios-beryllium/logs/usb-20261126-122550.txt`
(lines 3330-3343).

---------------------------------------------------------------------------------------------------------------------

## 11. Implemented: patch 1137 (stages 1 and 2 of section 7), NOT run on the phone

`xnu/patches-11215/1137-beryllium-slpi-stage1.patch` (on top of 1136), code in `bsd/dev/beryllium_wlan.c` (a new `sl_*`
section before `beryllium_wlan_init`, so the static FAT/SMEM/QMP/SCM helpers are reachable) and `bsd/dev/bergamota_rpmh.c`.
Built with `tools/xnu-local-build.sh --remake 15.3`, no errors; the image is
`research/ios18/kernel/kernel.development.bergamota-1137` (gitignored). Compile-checked only, never booted. No GLINK/QRTR, and
the `ipc` struct of the Wi-Fi stack is untouched.

Changes to existing code (the only ones): `ms_smem_alloc(item, size)` is now a wrapper of
`ms_smem_alloc_p(host_a, host_b, item, size)` (modem call sites unchanged); `bergamota_rpmh_arc_vote()` allows `lcx.lvl` and
`lmx.lvl` and, like `mss.lvl`, allows voting them off (cx/mx are still refused).

### Boot-args (all inert by default)

| Arg | Meaning |
|-----|---------|
| `bergamotaslpi=0/1/2` | 0 = nothing (no log line, no mapping). 1 = inventory. 2 = power + PAS boot. Needs `bergamotawlan=0` (shares `wl_fatbuf`/`wl_dirbuf` and `ms.*` with the Wi-Fi stage; refused with a log line otherwise). Higher values are refused. |
| `bergamotaslpidelay=S` | seconds before the stage starts (default 45) |
| `bergamotaslpipeek=0` | stage 1 only: skip reading the carve-out (default 1 = read 8 words) |
| `bergamotaslpipas=1` | stage 2: INIT_IMAGE + MEM_SETUP + the copy into 0x96b00000 (first SCM PAS call, first write there) |
| `bergamotaslpiauth=1` | stage 2: AUTH_AND_RESET, the only step that releases the DSP; needs `pas=1` |
| `bergamotaslpiwait=S` | seconds to wait for "ready" (default 10, max 60) |
| `bergamotaslpihold=S` | seconds to watch SMP2P after ready (default 20), then stop it |
| `bergamotaslpikeep=1` | leave the SLPI running after the hold (no stop, no unvote) |
| `bergamotaslpistop=N` | stop cleanly before step N of stage 2 (teardown runs) |

### Stage 1 (`bergamotaslpi=1`): log lines to expect (prefix `BERGAMOTA SLPI:`)

```
stage 1 starts in 45 s
stage 1: read-only inventory (no SCM, no vote, no write)
modem:image/slpi.b00 <n> bytes ... (one per file: slpi.mdt, slpi.bNN, slpir.jsn, slpius.jsn)
<n> slpi* files in modem:image/, <m> bNN segment files, <k> KiB in total      (or "(NONE: the SLPI firmware is not in this partition)")
slpi.mdt <n> bytes, ELF32 machine <n>, entry 0x..., <n> program headers
 ph<i> type .. off .. paddr .. filesz .. memsz .. flags ..[ HASH| (not loaded)]   (every header)
<n> loadable segments span 0x..0x.. (<n> KiB), RELOCATABLE|fixed address, FITS|DOES NOT FIT the carve-out 0x96b00000+0x1400000
image/slpi.bNN <n> bytes ok  ... / <n> segment files checked, <k> KiB, <b> bad
mdt verdict: loadable into the carve-out | NOT loadable (see above)
apps<->slpi partition at +0x15f000 size 0x30000: magic 0x54525024 hosts 0<->3 size ... free-uncached 0x20 free-cached ...
 no items (SLPI cold, nothing allocated yet)          (or "item N: bytes at +0x...")
smp2p item 481 absent (SLPI cold)                       (or the item header + entries)
crash reason item 424 absent
cmd-db lcx.lvl address 0x... (errno 0)   / cmd-db lmx.lvl ...
reading the first bytes of the carve-out 0x96b00000 (read-only)
carve-out +0: xxxxxxxx x8
stage 1 finished
```

Hang risk of stage 1: only the carve-out read, which is the last access, logged and flushed one second before it. Everything
else is FAT/SMEM/cmd-db reads that `bergamotawlan=3` already does. `bergamotaslpipeek=0` removes it.

### Stage 2 (`bergamotaslpi=2`): log lines to expect

Always (no sub-bit): steps 1-6, then undone.
```
stage 2: power + PAS 12 boot (pas=0 auth=0 keep=0 stop=0 wait=10 s hold=20 s)
step 1: map SMEM, TCSR mutex, AOSS QMP, APCS doorbells
step 2: slpi.mdt, segments and metadata from the modem partition (read-only)
   ... the stage-1 mdt lines, "segment files checked ... 0 bad", "metadata A + B = C bytes (hash ...)"
step 3: SCM availability (read-only)
SCM PAS INIT_IMAGE|MEM_SETUP|AUTH_AND_RESET|SHUTDOWN available: status=0 result=1 ; SCM PAS IS_SUPPORTED ... completed=1 status=0 result=1
PAS id 12 supported
step 4: AOSS QMP open + load_state slpi on
QMP magic 0x4d41494c version 1 ...  /  QMP "{class: image, res: load_state, name: slpi, val: on}" consumed     (these two lines carry the prefix "BERGAMOTA MSS:", the existing helper's)
step 5: RPMh ARC votes lcx/lmx to the top level
BERGAMOTA RPMH: lcx.lvl ARC 0x.....: N levels, vote hlvl N-1 ... ; same for lmx.lvl
step 6: SMP2P outbound item 430 ... + doorbell 26
smp2p item 430 ready at +0x...: version 1, pid 0->3, entry master-kernel, doorbell bit 26
stopping before PAS (add bergamotaslpipas=1)
teardown done              (votes off, load_state off, smp2p entries cleared)
stage 2 finished
```
With `bergamotaslpipas=1`, after step 6:
```
step 7: PAS 12 INIT_IMAGE (metadata) and MEM_SETUP
metadata <n> bytes at PA 0x...
SCM PAS INIT_IMAGE ... completed=1 status=0 result=0
SCM PAS MEM_SETUP ... completed=1 status=0 result=0
step 8: copy the segments into the carve-out (first write there), unmap
segment <i>: mapping 0x...+0x...   /   segment <i> slpi.bNN paddr .. file .. mem .. -> carve-out +0x... (<n> ms)   (each segment)
carve-out unmapped, <n> KiB loaded
image loaded, DSP NOT released (add bergamotaslpiauth=1)
SCM PAS SHUTDOWN ... completed=1 status=0 result=0          (from teardown)
teardown done
```
With `bergamotaslpiauth=1` as well:
```
step 9: PAS 12 AUTH_AND_RESET: releases the SLPI
SCM PAS AUTH_AND_RESET ... completed=1 status=0 result=0
signed image authenticated, waiting up to 10 s for ready
smp2p item 481 appeared after <n> ms        + item header and entries (version, pid 3->0, entries)
smp2p negotiated: version 1, features 0x1
smp2p in[0] "slave-kernel" = 0x2 (<n> ms)
SLPI READY after <n> ms
(hold: in[0] changes: 0x6 = handover (bit 2); on a crash 0x1 = fatal and  crash reason: "..." from SMEM item 424)
hold over: ready=1 handover=? fatal=0
stop request (master-kernel bit 0) / smp2p in[0] "slave-kernel" = 0x.. / stop-ack / stop-ack received
SCM PAS SHUTDOWN ... status=0 ; teardown done
```
Failure signatures: `AUTH_AND_RESET failed` (TZ rejected the image; teardown shuts down), `NOT ready in time` + whether item 481
ever appeared + crash reason, `fatal error before ready`. A silent phone with the last line `step N` names the step that hung
(every step flushes the log and waits 500 ms first).

### What can hang or reset (stage 2), and what guards it

1. Step 8, the first write into 0x96b00000..0x97f00000, gated by `pas=1`. Known precedent: the MBA carve-out next to it froze
   boots 32-35 because of a hidden `ml_io_unmap` panic on a permanent mapping. This code uses `ml_io_map_unmappable` per
   segment and unmaps it again right after the copy. Keep panics visible: never run with `bergamotafb_noconsole=1`.
2. Step 9, `AUTH_AND_RESET`, gated by `auth=1`: releases a signed Hexagon core. After it TZ/XPU owns the carve-out, and no
   mapping of it exists. An XPU violation or a rail/TLMM problem would be an instant reset, not a log line. User present.
3. Steps 4-5 (QMP, lcx/lmx at the top level) are new only in the resource names; QMP and ARC votes are the proven modem path.
   If `lcx.lvl`/`lmx.lvl` are not in cmd-db the vote fails with an errno and the stage tears down.
4. Step 6 is the first SMEM write into the apps<->slpi partition (hwlock path proven for apps<->modem).
5. `keep=1` leaves a running SLPI without GLINK servicing; its FIFOs are not set up yet, so it will idle waiting for
   a peer. A kernel restart by the watchdog (`bergamotawdt`) with it running is the unknown (the modem path always stops it).
6. The 10 s default wait is twice what Linux uses; the XO clock vote Linux makes is deliberately not made [G].
7. Not tested: no host harness exists for the new code (the FAT/mdt parsing is the modem's, unchanged in behaviour).
