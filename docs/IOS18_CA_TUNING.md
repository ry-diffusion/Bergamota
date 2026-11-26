# iOS 18 CoreAnimation tuning knobs (from the 22H352 shared cache, 2026-11-26)

Offline study of `research/ios18/dsc` (iOS 18.7.8 22H352, PAC-stripped cache) with `tools/renegade/dsctools`
(`imgstr.py`, `xref.py`, `dcache.py` with `DSC_OBJC=0` for speed) plus a pointer scan of QuartzCore's `__DATA_CONST`.
Goal: steadier and cheaper frames on Crimson/Turnip without code changes. Nothing here has been measured on the phone
yet; each knob is an experiment for the next session (method: NOTES, "Clean measurement").

## How QuartzCore reads its settings

- QuartzCore knows 307 `CA_*` strings; 268 of them are runtime options in one table of name pointers at
  `0x1e7316c60` (`__DATA_CONST`; dump with scratchpad `table.py`).
- The initializer (QuartzCore+0xea8a0) walks it:
  - Indices 0-198 are booleans, stored one byte each at `0x1ed835318 + index`.
  - Indices 199-267 are floats, at `0x1ed8351c0 + 4 * (index - 199)`.
  - Each value comes from a lookup (QuartzCore+0xef6d4) that reads the environment variable and, only when a flag
    byte at `0x1ed8352e8` is set (looks like internal builds), a `CFPreferencesCopyMultiple` dictionary.
- **On this phone, set them as environment variables:**
  - backboardd: lines `NAME=VALUE` in `/private/var/tmp/crimson-bb.env` (the inject `setenv`s every line before CA starts;
    needs a reboot, backboardd is never restarted by hand).
  - SpringBoard: `tools/bergamotanet/sblaunch.c` (add `setenv` lines; it execs SpringBoard).
- `CAGetDebugOptionEnvVariableName` (exported) maps an index back to its name.

## Candidates, ranked

| # | Knob | Index | What the code shows | Expected effect on Crimson/Turnip |
| --- | --- | --- | --- | --- |
| 1 | `CA_DISABLE_INPLACE_SHADERS=1` | 136 (bool) | Feature query (QuartzCore+0x6ad94, switch over 41 features): in-place = `!option && ctx byte 0xab5 bit 5` (a device capability). | In-place shaders read the destination (framebuffer fetch). Crimson answers the GPU families that include programmable blending, so CA uses them, and Crimson emulates fetch with input attachments, which keeps Turnip passes in GMEM (iOS 26: 22 CA shaders fetch). Off = CA's non-fetch path: maybe sysmem-friendly, maybe an extra copy. **A/B: GPU ms/frame, passes/frame, fps.** |
| 2 | Reduce Transparency: `com.apple.Accessibility` `EnhancedBackgroundContrastEnabled` (libAccessibility `_AXSSetEnhanceBackgroundContrastEnabled`) | n/a | System-wide setting (Settings > Accessibility > Display & Text Size) | Blur backdrops become flat fills: fewer passes and draws in the App Library, folders, Control Center and Dock. Visible change, so the user decides. |
| 3 | `CA_ENABLE_PREFERS_TRIPLE_BUFFERING=1` | 261 (float) | read as a float option | Deeper swap queue: steadier pacing with a frame of extra latency. Pairs with the kernel's second swap slot (1027, `kern.bergamota_fb_queue2`). |
| 4 | Reduce Motion: `ReduceMotionEnabled` (+ `ReduceMotionReduceSlideTransitionsPreference`) | n/a | libAccessibility keys | Cross-fades instead of zooms and parallax: less CA work in transitions; page drags unchanged. Visible change. |
| 5 | `CA_BACKDROP_SCALE` (e.g. 0.25) | 202 (float) | read at QuartzCore+0x4f578 area with the backdrop cache | Smaller blur textures: cheaper blurs, softer look. (iOS 26 capped CABackdropLayer.scale at 0.5 in SpringBoard: negligible there.) |
| 6 | `CA_DISABLE_LOW_LATENCY=1` | 38 (bool) | read in the backing-store path (QuartzCore+0x355ac) and `CASetDebugFlags` | Lets CA queue instead of rushing low-latency frames; may steady pacing. |
| 7 | `CA_MINIMUM_FRAME_DURATION` | 249 (float) | read at QuartzCore+0x2850 / +0x2878 / +0x47eb4 / +0xe84f0 (display timing) | A frame-duration floor; could pin a steady rate (e.g. 1/60) instead of letting timing drift. |
| - | Do not touch | | `CA_DISABLE_TILE_SHADERS` (135: Crimson already declines tile shaders), `CA_DISABLE_GENERIC_SHADERS` (86), `CA_DISABLE_OCCLUSION_CULLING` / `CA_DISABLE_HIDDEN_CULLING` (culling saves work), `CA_DISABLE_FILTER_MERGING` (merging saves passes). | |

Also seen:
- `CA_HEARTBEAT_RATE` (Jesvi's 120 Hz lever).
- `CA_DISABLE_DEFERRED_ANIM_CALLBACKS`, `CA_DISABLE_ENCODER_SHARING`, `CA_DISABLE_PRECOMPILED_PIPELINES`.
- `CA_ENABLE_FRAME_RATE_ARBITRATION`, `CA_FRAME_RATE_POLICY`.
- SpringBoard's stock launchd job sets `CA_ASSERT_MAIN_THREAD_TRANSACTIONS=1` (a cheap check, left alone).

BiomeStreams/BiomeStorage have no kill switch; their "DISABLE" strings are SQL. The `ufswait=1` fix already took most of
Biome's CPU cost during transitions.

## Next phone session

1. One variable at a time in `crimson-bb.env`, reboot, `perf/run2.sh` pages + applib with the phone unlocked, 2 runs each, plus the
   jitter summary (`perf/jitter.py`).
2. Measure continuous drags through `fgjob` (`tools/bergamotanet/fgjob.c`, build recipe in its header). SSH-started injectors run
   darwin-background and stutter, which inflates jitter. The `perf/run2.sh` numbers so far are SSH-driven.
