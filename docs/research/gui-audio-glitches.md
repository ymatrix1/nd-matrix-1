# GUI audio glitches: the firmware FX racks exceed the block budget

- **Checked:** 2026-11-17 (`main` @ `666dace1`)
- **Sources:** `ns4 bench --gui` (new `--backup`, `--program`, `--arp`, `--repeat`, FX summary), `crates/nord-gui/src/audio/{callback,meter,io}.rs`, `crates/nord-core/src/engine/mix.rs`, `fx/reverb.rs`, `fx/fw_rvbm*.rs`, `fx/fw_ext40.rs`, `fx/fw_leslie_core.rs`, `fx/master.rs`, a `samply` sampling profile symbolised from `target/release/ns4.pdb`. The factory program was read from the local `.reference/` backup; nothing from it is committed.
- **Question:** The GUI on program A:16 Horizon Whl with `Engine [tone=Firmware filter=Firmware fx]` showed `late 9569/189418, worst 75.81/10.00 ms, load 34%, peak -2.4 dB`, and the player hears clipping and over-budget blocks. Which stage causes it, and is it the render or the host?

## 1. Reproduction (no sound card)

```bash
cargo run --release -p nord-core --bin ns4 -- bench --gui --layers 3 --frames 480 --keys 6 \
  --arp up --warmup 450 --blocks 150 --repeat 3 \
  --backup ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" \
  --program "Program/Bank A/Horizon Whl.ns4p" \
  --tone-source firmware --filter-engine firmware --firmware-fx
```

`--repeat N` renders the (deterministic) run N times and keeps each block's fastest time, which removes scheduler noise from other processes (this host runs several jobs, a 16 core CPU, so single runs vary by 20 to 40 percent). Without it the p99 and max of a run are mostly the host: the same Scene II run showed a 37 ms worst block single, and a 9.4 ms worst block (mean 6.0, p99 7.8, none late) after min of 3. **The tail spikes of the status bar are not a render stage; the render is over budget on average for some programs and the host adds the rest.**

The bench prints what each rack runs (`FX racks:`). Horizon Whl: **all seven racks run a firmware `Cathedral` reverb at mix 0.34**, Piano A/B and Synth C add a `Vibe`, Synth C a delay.

## 2. Findings

| Finding | Evidence |
|---|---|
| The FX racks are the cost, not the layers. Horizon Whl, Scene I, 480 frames: default FX 0.64 ms, `--firmware-fx` 12.7 ms (all ports). Ports alone: delay laws + time 1.2, compressor 1.4, Mod 1 + Mod 2 2.9, Amp EQ 3.7, **reverb 7.6** (ms per 480 frame block, min of 3) | `--firmware-fx-ports ...` rows |
| Each firmware reverb is an instruction-level SHARC model (`fw_rvbm_steps_*.rs` on the register machine of `fw_leslie_core.rs`, `E40` 40 bit arithmetic): about 1 ms per rack per block. The engine runs **all seven racks every frame**, also for layers that are not sounding (the other scene, unused layers) | profile: 52 percent of the render inside `ReverbMulti::process_frame`; `mix.rs` loops `fx_chains` over all 7 |
| `E40::add` / `mul` used 128 bit integer arithmetic for every operation | profile: `E40::add` 6 percent self, `pack` inlined in the steps |
| No allocation or lock on the audio path: the lazy `FwReverb::new` / `FwReverbMulti::new` (first use, type change) costs 2 to 37 microseconds, the callback has no mutex (`callback.rs`), the meter is atomics. Flush-to-zero and key churn were ruled out earlier ([gui-audio](../specs/gui-audio.md)) | measured |
| The master bus **hard clamps at plus or minus 1.0** (`fx/master.rs` `clamp(-1.0, 1.0)`, before the 0.85 standalone volume): a loud default chord (10 to 88 keys, all layers) peaks at exactly 0.850. Horizon Whl itself peaks at 0.09 to 0.13 in the bench, so its crackle is block overruns (a late block repeats or drops a buffer), not level. The meter's `over_0db` cannot see a clamp, because the clamp sits before it | bench peak 0.850 for the default engine; 0.084 to 0.132 for Horizon Whl |
| NaN / Inf: no non-finite output in any bench run; the meter counts NaN as over | bench peaks finite |

## 3. What changed

| Change | Output | Default |
|---|---|---|
| `E40::add` takes an exact-f64 fast path (two-sum error term zero, then round to 32 bits by integer ops), `mul` uses a 64 bit pack | bit-identical: the oracle goldens (`tests_fw_rvbm_golden`, `tests_fw_rvb_golden`, Leslie) pass and a 200 000 pair random test compares against the old integer path; `render_hash` of the bench identical before and after (`0x47576812a4ed6e44` Scene I, `0x1685b873c8315384` 150 blocks) | on |
| `--fx-idle-skip` (GUI "Skip idle FX racks"): a rack whose input has been exactly 0 and whose output stayed under 1e-7 (-140 dBFS) for 4 s returns silence until input returns | the hash of a render in which every sounding layer keeps playing is identical (test); a rack that wakes up resumes from frozen state, so a later wake is not bit-identical to the always-run path | off (D161) |
| `ns4 bench --gui`: `--backup/--program/--arp/--repeat`, `FX racks:` line, `breakdown` on the loaded program | tool only | |

## 4. Results

Horizon Whl, Scene I, 480 frames, 6 keys + arpeggiator, `--firmware-fx`, min of 3 (6 for the second pair), one noisy host (other jobs run on it; rows of one table are comparable, runs differ by plus or minus 10 percent). Mean ms per block against the 10.0 ms deadline, `render_hash` equal in every row of a pair:

| Build | mean | p99 | late of 150 | hash |
|---|---|---|---|---|
| before (original `E40`) | 15.20 | 17.34 | 150 | `0x47576812a4ed6e44` |
| `E40` fast path | 14.41 | 16.13 | 149 | `0x47576812a4ed6e44` |
| `E40` fast path + `--fx-idle-skip` | 3.80 | 4.40 | 0 | `0x47576812a4ed6e44` |
| before, repeat 6, 100 blocks | 12.68 to 13.06 | 14.6 to 15.0 | | `0x1685b873c8315384` |
| `E40` fast path, repeat 6 | 12.12 to 12.35 | 14.0 to 14.3 | | `0x1685b873c8315384` |

The `E40` change is a bit-identical 4 to 5 percent. The idle skip is a factor of 4 on a Scene I program because four of the seven racks (Organ B, Piano B, Synth B, Synth C) never get input. A Scene II program that plays all seven layers gets nothing from it (not measured: `--layers 7` only switches the scene flag, the loaded program decides which layers sound). Engine (non firmware) FX of the same program: 0.64 ms.

## 4b. Why the GUI said `load 34%, 5% late` and the bench says 12 to 15 ms

Both cannot describe the same render. Not verified (no GUI trace of the owner's session exists): the GUI run most likely did not have Horizon Whl's seven firmware Cathedral reverbs active. Evidence: the same bench **without** a loaded program (default racks: engine reverb `Stage` mix 0.15, `--firmware-fx` on, Scene II, 10 keys, 480 frames) gave mean 5.9 ms (59 percent) in a single run on this host, p99 14.7, 149 of 3000 late (5 percent), no keys held 3.4 ms (34 percent). That matches the GUI numbers (34 to 59 percent load, 5 percent late, one worst block far above the mean) and is host jitter on a render that is itself 30 to 60 percent of the budget. Loading Horizon Whl through the bench (`apply_program` with its FX words) takes the same flags to 12 to 15 ms. So the possible causes of the gap are: a different FX state in the GUI (the program's FX not applied, or applied later through another path), a faster host at the time, or both. What to check on the GUI machine: run `nd-matrix-1` with `--fx-idle-skip` off and on and compare the `load` figure after loading A:16; if load stays near 34 percent with the program loaded, the GUI program-load path does not enable the program's reverb racks and the bench overstates it.

## 5. Guidance

- Scene II programs that play all seven layers with firmware reverbs need the reverb itself to get cheaper; the idle skip does not help them. `--audio-buffer 960` or 1440 gives the callback 20 to 30 ms and hides over-budget blocks at the price of latency (device buffer size, not the render block).
- Real-time safe: nothing in the opt-in or the fast path allocates, locks or does I/O.

## Implications

The glitches are the firmware FX racks running at about 1.3 times the block budget on a scene with firmware reverbs, plus host scheduling on top. A hard clip at the master bus exists for loud default chords.

## Follow-ups

- Reverb cost: the register machine does a linear window search per memory access (`Core::slot`) and a `V` enum per register element; a specialised straight-line f32 path would need an error bound against the oracle (opt-in, `FwReverbMultiF32` already exists).
- Share one reverb between racks with identical settings (the program stores one reverb for the layers? needs the firmware evidence), then default the idle skip once the owner agrees.
- Master bus: an opt-in soft limiter instead of the hard clamp, and count clamped frames in the GUI meter.

## 6. Round 2 (2026-11-19, D167): the crackle that survives D161 with the new defaults

- **Checked:** 2026-11-19 (`main` @ `6a8ba992` plus this branch). **Sources:** `nd-matrix-1 --audio-probe` (new: the real cpal callback without a window), `ns4 bench --gui --reload` (new), cpal 0.15.3 `host/wasapi/stream.rs`, Windows 11, Ryzen 9 5900XT (16 cores), default output device 48 kHz F32, driver default buffer 480 frames (10 ms). **Question:** after D161 and the heavier defaults (firmware Mod 2, Touch, arp walk, morph), what still makes the GUI crackle?

### 6.1 Findings

| # | Finding | Evidence |
|---|---|---|
| 1 | **The callback thread runs at normal priority with no MMCSS class.** cpal's `boost_current_thread_priority` calls `SetThreadPriority(HANDLE(GetCurrentThreadId() as isize), TIME_CRITICAL)`: a thread id is not a handle, the call fails and the result is discarded | probe: `thread priority 15 (was 0)` (`GetThreadPriority` before our call reads 0 on cpal's thread) |
| 2 | Under CPU contention the unboosted callback loses blocks even when the render is cheap (Scene II, 88 keys, default engine, 26 percent load): 13 late blocks, worst callback 20.9 ms, 11 callbacks started more than 1.5 periods late | table 6.2, row C |
| 3 | The default configuration is **not render bound** on this host: A:16 Horizon Whl Scene I with arp 25 to 30 percent load, 0 late idle; the default start-up engine in Scene II with 88 keys 25 to 26 percent | table 6.2, rows A, C |
| 4 | `--firmware-fx` (opt-in) is render bound on Horizon Whl: 86 to 88 percent load, 115 to 198 of about 1200 callbacks late with an idle host, and a boost cannot fix a render that is longer than the buffer | table 6.2, row B |
| 5 | **The master bus hard clamp is audible as distortion on hot patches.** The default start-up engine in Scene II clamps 34 151 of 576 000 frames (5.9 percent) of the probe chord (88 keys) and `ns4 bench --gui --layers 7 --keys 10` clamps 13 percent of the frames (Scene II 4 keys: 2 percent, Scene I 4 keys: 0). The meter's over-0-dBFS count stays 0 because the clamp sits before it; the meter now counts `clamped` frames | probe row C `clamped 34151`; bench line `master clamp hit` |
| 6 | Program load is not a timing problem with these defaults: `apply_program` costs 0.007 to 0.011 ms on average and 0.3 to 0.8 ms at worst; the first block after a load is 0.03 to 0.07 ms slower than the next two on average (0.45 to 1.4 ms worst) on 128 frame blocks, so lazy stage builds are real but small. It was still an allocation and a free on the audio thread (a program box, Mod 2 stage boxes, a replaced sound bank), now off it | `--reload 300 --reload-pool 59`; counting allocator test |
| 7 | Host scheduling spikes (the 9 to 17 ms of the brief) are the same effect as finding 1 | rows C and B under contention |

### 6.2 Real callback (`--audio-probe 12`, driver default buffer 480 frames), before = `--no-thread-boost`, after = boost on

Contention = 40 busy-loop processes (a Python `multiprocessing` spin loop) at normal priority on the same host. Blocks is the callback count in 12 s (1200 when nothing starves; fewer means the device ran dry and callbacks slipped). Render, firmware flags and chord are identical in a row pair; only the thread class differs.

| Case | Host | Boost | Blocks | Late | Worst callback | Slow gaps | Load |
|---|---|---|---|---|---|---|---|
| A Horizon Whl defaults, Scene I, 6 keys, arp up | idle | before | 1201 | 0 | 6.05 ms | 0 | 30% |
| | idle | after | 1200 | 0 | 4.93 ms | 0 | 25% |
| | contention | before | 1200 | 0 | 8.39 ms | 0 | 37% |
| | contention | after | 1201 | 0 | 5.63 ms | 0 | 37% |
| B Horizon Whl `--firmware-fx` | idle | before | 1194 | 115 | 16.73 ms | 4 | 86% |
| | idle | after | 1176 | 198 | 16.90 ms | 16 | 88% |
| | contention | before | 807 | 705 | 68.49 ms | 330 | 138% |
| | contention | after | 899 | 678 | 23.86 ms | 246 | 125% |
| C default engine, Scene II, 88 keys | idle | before | 1200 | 0 | 5.49 ms | 0 | 26% |
| | idle | after | 1201 | 0 | 6.10 ms | 0 | 25% |
| | contention | before | 1204 | **13** | **20.94 ms** | **11** | 38% |
| | contention | after | 1207 | **0** | **4.47 ms** | **0** | 36% |
| D Horizon Whl `--firmware-fx --fx-idle-skip` | idle | before | 1200 | 24 | 14.35 ms | 0 | 33% |
| | idle | after | 1201 | 0 | 9.83 ms | 0 | 35% |
| | contention | before | 1048 | 247 | 43.18 ms | 128 | 63% |
| | contention | after | 1084 | 233 | 20.83 ms | 105 | 55% |

Reading: the boost removes the scheduler losses (C: 13 to 0 late, 20.9 to 4.5 ms; B and D under contention: worst callback 68 to 24 and 43 to 21 ms); it cannot help when the render alone is longer than the buffer (B idle: 115 and 198 late, the same render, the difference is host noise). D idle "before" shows 24 late because the 4 s sleep timer of the idle skip is still running during the first two measured seconds; the boost row is 0 late. `--fx-idle-skip` under contention stays late (233 of 1084): the racks that sound still cost 5 to 6 ms and the busy processes share the cores with them. **Proposal for the owner: make `--fx-idle-skip` default when `--firmware-fx` is on.** Evidence: row B to D on the idle host, 86 percent load and 115 to 198 late to 33 to 35 percent and 0 to 24 late (the output hash is equal while no rack wakes, not bit-identical once a sleeping rack wakes, D161).

### 6.3 What changed

| Change | Output | Default |
|---|---|---|
| `priority::boost_current_thread` (TIME_CRITICAL + MMCSS `Pro Audio`) on the first callback; `--no-thread-boost` opts out | none (scheduling) | on |
| Meter: callback gap, slow gaps, clamped frames, thread class; `MasterBus::clamped_frames` | bit-identical (`ns4 bench --gui --layers 3 --keys 4 --frames 480 --blocks 100 --warmup 100` prints hash `0x550c3621bf91cf88` before and after) | on |
| `garbage.rs` queue and `ns4-reaper`: the callback no longer frees program boxes or replaced sound banks | none | on |
| `reserve_firmware_stages`, in-place stage re-init (TD-18) | bit-identical (`fx::tests_reserve_stages`) | on in `standalone_engine` |
| `nd-matrix-1 --audio-probe`, `ns4 bench --gui --reload / --reload-pool / --program2`, master clamp line in `ns4 bench --gui` | tools | |

## Implications (round 2)

The crackle with default settings has two causes that no bench shows: an unprioritised callback thread (fixed) and the master bus hard clamp on loud patches (measured, not changed). A render longer than the buffer (`--firmware-fx` on seven reverbs) stays over budget; idle skip is the cheap lever and needs the owner's decision.

## Follow-ups (round 2)

- Opt-in soft limiter (or a higher-ceiling knee) instead of the hard clamp, measured with the new `clamped` counter on real programs; default flip is the owner's call.
- `--fx-idle-skip` default with `--firmware-fx` (numbers above).
- Render-ahead thread with an adaptive ring (renders in a high-priority thread, the callback only copies): hides a render that is longer than one buffer at a latency cost. Not done: the boost fixed the measured losses and `--audio-buffer 960` gives the same headroom today.
- The plugin does not call `reserve_audio_stages` or raise a thread (the host owns the audio thread).
