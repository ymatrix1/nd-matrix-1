# GUI audio callback and underrun meter

Checked: 2026-11-17. Code: `crates/nord-gui/src/audio/{callback,meter,meter_ui,io}.rs`, bench `crates/nord-core/src/bench_gui*.rs`.

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
