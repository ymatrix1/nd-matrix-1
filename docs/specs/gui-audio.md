# GUI audio callback and underrun meter

Checked: 2026-11-19. Code: `crates/nord-gui/src/audio/{callback,meter,meter_ui,io,priority,garbage,probe}.rs`, bench `crates/nord-core/src/bench_gui*.rs`.

## Callback

`io.rs` opens the default CPAL output with the driver's default buffer (`--audio-buffer <frames>` forces a fixed size). `AudioCallback::run` (`callback.rs`) does, per device buffer:

| Step | Work |
|---|---|
| drain | `rx.try_recv()` loop, `apply_audio_command` (control ops that would allocate are dropped and counted) |
| render | `Stage4Engine::render_frame()` per frame, x master volume 0.85, written as F32 or I16 |
| publish | `publish_telemetry()` (seqlock write) |
| record | `AudioMeter::record` |

The GUI path does not call `begin_block` (extern volume sync, FX / organ sharing), which `process_block` and `process_block_multi` run once per block. Known difference, not changed here (it would change the sound).

## Meter

`AudioMeter` is atomics only (no allocation, lock or I/O); the callback is the single writer.

| Field | Meaning |
|---|---|
| late blocks | callback time (drain + render + publish) greater than `frames / sample_rate` |
| mean / worst ms | callback time; worst drain and worst publish are kept separately |
| load | total callback time over total audio time (sum of `frames / sample_rate`) since reset, so it stays right if the device changes the buffer size |
| peak, over 0 dBFS | post master-volume sample magnitude; count of samples with abs > 1 or NaN |

The GUI shows it in the bottom bar (`late n/blocks  worst x/deadline ms  load  peak`), amber above 70 % load or over 0 dBFS, red after a late block; Reset asks the audio thread to zero the counters at its next block. The summary line is also printed to stderr on exit. A late block is a lower bound on a dropout: the driver may hold more than one buffer.

## Findings (bench `ns4 bench --gui`, GUI default options, 128 frames, 48 kHz, one noisy Windows host)

| Workload | mean | p99 | max | late of 4000 |
|---|---|---|---|---|
| 7 layers, 88 keys | 1.2 to 1.4 ms | 2.0 to 2.5 | 3 to 5 | 7 to 21 |
| 3 layers, 20 keys | 0.5 to 0.65 ms | 0.9 to 1.0 | 1.4 to 2.1 | 0 |

- Flush-to-zero was tried (MXCSR FTZ+DAZ): output hash identical on and off (static chord and key churn), no timing change. No denormal cost, so no FTZ in the app.
- Voice start: `--churn 8` (release the oldest held key, press a free one, every 8th block; 7 layers, 88 keys, 200 warm-up blocks, 3000 blocks, same noisy host, three runs) gave press blocks mean 1.80 / 1.81 / 2.05 ms against 1.80 / 1.76 / 2.06 ms for the others, so a voice start adds no visible mean cost. Worst press block was 3.4 to 8.2 ms, in the range of the worst block of runs without churn (6 to 34 ms on this host), so one voice start is not isolated as a spike source either; the sample is small and the host was loaded. An earlier version of the bench pressed keys that were already held (fixed: the chord is tracked and keys stay inside the 21..=108 keybed).
- Worst block index changes from run to run, also with no key held or one layer, so tail spikes are host scheduling, not a render stage. Remaining lever: device buffer size (`--audio-buffer`; a size the device rejects is reported and the default buffer is used instead, a size outside the supported range is clamped).
- **Firmware FX load (2026-11-17, D161):** the tail of the status bar is not a render stage, but the render itself can exceed the buffer: with `--firmware-fx` a program whose seven racks run a firmware reverb (A:16 Horizon Whl) costs 12 to 15 ms per 480 frame block, against 10 ms of audio. `ns4 bench --gui --backup <.ns4b> --program <entry> --arp up --repeat 3` reproduces it (`--repeat` keeps each block's fastest time to remove host noise); `--fx-idle-skip` stops the racks of silent layers (opt-in), see [gui-audio-glitches](../research/gui-audio-glitches.md). The master bus clamps hard at 1.0 before the meter, so the meter's over-0-dB count cannot see a clipped chord.
- Device seen: 44.1 kHz stereo F32, driver default buffer (size not reported by CPAL; the meter shows the real callback size).

## Thread class, gap meter, clamp counter, garbage queue (D167, 2026-11-19)

Code: `audio/{priority,garbage,probe}.rs`, `audio/callback.rs`, `fx/master.rs`, `fx/modulation.rs` (`reserve_firmware_stages`), bench `bench_gui_reload.rs`.

| Piece | Behaviour |
|---|---|
| Thread priority | The first callback calls `priority::boost_current_thread`: `SetThreadPriority(GetCurrentThread(), TIME_CRITICAL)` and the MMCSS task `Pro Audio`. cpal 0.15.3 (`host/wasapi/stream.rs`, `boost_current_thread_priority`) passes the thread id as the `HANDLE`, so its own boost fails silently; a probe on this host read priority 0 and no MMCSS before the call. The boost is one system call outside the first block's timing. `--no-thread-boost` skips it (comparison runs). Other hosts: no-op |
| Meter: gap | `BlockSample::gap_ns` is the start-to-start time of two callbacks. The device paces them, so a gap over 1.5 periods (`slow_gaps`) means the thread was not scheduled in time, whatever the render cost. `worst_gap_ms` is the longest |
| Meter: clamp | `MasterBus::clamped_frames` counts frames whose summed bus exceeded plus or minus 1.0 before the hard clamp (the output is unchanged). `BlockSample::clamped` is the per-block delta; the summary prints `clamped N`. The over-0-dBFS count cannot see the clamp because it sits after it |
| Meter: thread | `ThreadBoost` (priority before and after, MMCSS) is packed into one atomic and printed in the summary |
| Garbage queue | `LoadProgram` (`Box<ProgramDecoded>`), `LoadPianoSound` and `LoadSynthSample` (the replaced `Arc`) no longer free on the callback: `apply_realtime` puts them into a bounded queue (`try_send`, no allocation, never blocks, a full queue leaks) and the `ns4-reaper` thread drops them |
| Stage boxes | `standalone_engine` calls `Stage4Engine::reserve_audio_stages`: every firmware Mod 1 / Mod 2 stage box exists from the start and a type change re-initialises a box in place (the constructors build on the stack). Output is bit-identical (`fx::tests_reserve_stages`); `tests/no_alloc_fx_stages.rs` counts 0 allocator calls for ten program loads with different Mod 1 / Mod 2 types, against more than 20 on an engine that does not reserve |

### Real callback probe

`nd-matrix-1 --audio-probe <secs> [--backup <.ns4b> --program "Program/Bank A/Horizon Whl.ns4p"] [--arp up] [--keys 6] [--layers 3|7] [--audio-buffer N] [--no-thread-boost] [firmware flags]` opens the default output device like the GUI (same `AudioCallback`, buffer and boost flags), plays the bench engine for `<secs>` after 2 s of warm-up and prints the meter. It makes sound and needs a device. `ns4 bench --gui` times the render alone; the probe adds the driver, the thread class and the scheduler.

### Program reload bench

`ns4 bench --gui --backup <.ns4b> --program <entry> --reload N [--reload-pool K] [--program2 <entry>]` plays a chord, applies N programs in turn (the named ones, then K programs spread over the archive) with `apply_program` and times the call and the three blocks after it: the cost of a program change inside the callback and of the stages it builds lazily.
