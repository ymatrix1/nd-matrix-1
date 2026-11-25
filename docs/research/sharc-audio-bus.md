# Multi-chip LLE: FX cores across passes, the delay window and the audio bus

- **Checked:** 2026-11-24
- **Sources (local, never committed):** the FX, ORG and PNO DSP images (`target/DSP/`, flat SRAM through `python tools/ns4_dsp_ldr.py --sram`), the ARM load scripts of `target/org_system5/` (`fx-sys-load-core1.script`, `fx-sys-load-core2.script`, `org-sys-load-core2.script`), `tools/fx_oracle_live.py identity`; committed: [pno-jit-arm-profile](pno-jit-arm-profile.md), [fx-oracle-live](fx-oracle-live.md), [fx-dsp-map](fx-dsp-map.md), [dsp-peripherals-audio](dsp-peripherals-audio.md), [sharc-lle-realtime](sharc-lle-realtime.md), [sharc-sdram-window](sharc-sdram-window.md); code `runner/{fx_run,pass_sched,audio_bus,audio_bus_node,audio_bus_sy,lle_system}.rs`, `crates/nord-tools/src/bin/lle_sharc_bus.rs`.
- **Question:** can both FX cores run across arbitrary pass counts in the scheduler, and can the audio of the tone chips (ORG, PNO, SY1SY2, SMP) reach the FX cores and a host stereo buffer without a WAV hand-over (bean `ns4-lle4`)?

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| FX Core2 "falls off its poll loop after pass 0" | Not a Core2 bug: pass 1 waits at `0x1212E4` for the Core1 hand-over counter `DM(0x282C0504) & 7 == DM(0xB0141) - 1`, and no harness presented it (`sharc-sys` and `lle_sharc_bench` do not call `FxPeer`). `FxDrive::before_pass` now plays the counter and the ping selector before every pass in `fx_live_run`, `PassJob` (so `pass_sched` and `lle_sharc_bench`) and `CoreNode` | VERIFIED (runs below) |
| Both FX cores over 316 passes | `lle_sharc_bench --core fx1=...,fx1,SCRIPT --core fx2=...,fx2,SCRIPT`: every pass returns to the poll loop, FX Core1 3,370 steps/pass, FX Core2 3,220 steps/pass (the figures of `pno-jit-arm-profile.md`), interpreter 182 us and 188 us per pass (x3.95 and x4.08 of the 46.08 us budget) | VERIFIED (run, noisy host) |
| Before the change | `sharc-sys` FX Core2, same script: pass 0 3,220 steps, pass 1 hits the 200,000-step cap | VERIFIED (run) |
| External delay window | dense `0x0800_0000..` store, [sharc-sdram-window](sharc-sdram-window.md) | VERIFIED mapping |
| Audio bus | `AudioBus` over `BusNode`s: tone chips out, FX cores in, host buffer; one pass of latency per hop, FIFO per input, sums with gains, deterministic and independent of node order | model ASSUMED (below), code tested |
| Bus against the direct run | FX Core1 with the `identity` stage script and a 1.5 s burst, 17,000 passes: the host WAV of `lle_sharc_bus --node fx1=...,fx1,identity.script --route fx1>host --in-wav fx1:in.wav` is **byte identical** to the WAV of `sharc-fxlive --core 1` | VERIFIED (`cmp`) |
| Three real chips on the bus | ORG Core2 (`org` preset, load script plus `0 0x3e021 1`) -> FX Core1 -> FX Core2 -> host, 400 ticks, all passes idle, host silent (no key and no effect installed: the ARM key messages for ORG were not captured, `org-sys-key1-core2.script` is empty) | run; **no audio claim** |

## 2. The FX hand-over (item 1)

`fx_run.rs` already contained the answer (`FxPeer`, D122): Core2's pass starts with a wait for the counter Core1 keeps (`0x1212E4`, `fx-oracle-live.md` s2). `sharc-fxlive` called it, the scheduler paths did not. `FxDrive` bundles the two per-pass writes (`peer.before_pass` for Core2 only, `DM(ping_select) = PING_VALUE` for both) so one definition serves three callers. `PassJob::before_pass` now holds everything a job does before pass `p` (ping-pong pointer, due messages, FX drive, input refresh) and both `run_ticks` and `CoreNode` call it. The presets `fx1` and `fx2` (`live_presets.rs`) carry the descriptor, so `lle_sharc_bench` and `lle_sharc_bus` take them by name; the ARM load scripts stay local (firmware-derived, D20/D128) and are read with `parse_msg_text` (flat `PASS W1 W2` lines, all delivered before pass 0).

## 3. The bus model (item 3)

The transmit side of the SPORT DMA never appears as a DSP access (`dsp-peripherals-audio.md`), so, like the ARM messages, the transfer is modelled by its effect. Everything about the topology is ASSUMED and kept as data of the caller:

| Aspect | Model | Evidence |
|---|---|---|
| Frames per pass | tone chip 2 (PNO lane pair, 2 ISRs of SY/SMP), FX 4 | VERIFIED (`pno-full-pass.md` s1, `fx_run.rs` `FRAMES_PER_PASS`) |
| Tick | the 2-frame period; an FX core runs every 2nd tick | ASSUMED (the FX pass period was not measured against the tone chips) |
| Latency | one tick per hop; a node sees its inputs from the tick after they were produced | ASSUMED (the DMA double buffer depth is unknown) |
| Start-up | a consumer's FIFO is zero padded and the shortfall counted (`underruns`) | model |
| Mixing | routes into one input add sample by sample times their gain | ASSUMED (the PNO block holds the sum of three sections already, `fx_oracle_live.py` BUS comment) |
| Which chip feeds which FX input, Core1 -> Core2 | routes given on the command line, none by default | ASSUMED, open |

Output taps (`OutTap`): a block at an address, a block behind a pointer cell (FX Core2 `DM(0xC03D7)`), an ORG/PNO lane pair (`DM(0xB0159 + set)`, VERIFIED layout), the SY/SMP tapped frame of `SyModel` (D153/D200/D235). Input: the 4-frame block at `FxCore::input_base` (`0xC01BC` Core1, `0xC0180` Core2).

## 4. Not done

- Real tone-chip audio through real FX stages: needs the ARM key and weight messages (`pno-jit-arm-profile.md`, follow-up 1) and an effect chain installed on the FX cores; the run above proves structure and the exactness of the hop, not a mix.
- The threaded scheduler (`pass_sched::run_ticks`) does not exchange frames; the bus runs the nodes in one thread (correctness first, `sharc-lle-realtime.md` NO-GO on real time). Because a hop costs a full tick, the exchange is order independent and can move to the tick barrier later.
- SY1SY2 and SMP on the bus use `SyModel` and are unit-tested with a bus tap only (no SY/SMP image run here; those chips need their flash/EP-DMA models, D141, `--epchain`).
- Peripheral gaps of `ns4-36hw` (`0x28xxxxxx` core aliases, `SIMD + LW`): not hit by any run above, so not touched.

## Implications

- The `FX Core2` blocker of `sharc-lle-realtime.md` is closed; FX cores join the scheduler with presets, and a 5-chip benchmark only needs the missing message scripts (SY, SMP are interrupt driven, not `pass_sched` jobs).
- A system with FX in the loop costs the FX passes on top of the tone chips: about x4 of the budget each on this host's interpreter (not real time).

## Follow-ups

1. Capture the ARM key and effect-install messages for a held note, then null the bus output against the HLE engine.
2. Measure the tone-chip to FX frame offset (DMA depth) once a bus capture exists.
3. Move the frame exchange to the tick barrier of `run_ticks` so the threaded benchmark also carries audio.

## Reproduce

```bash
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram FX      # and ORG
python tools/fx_oracle_live.py identity --image FX/core1_sram.bin --out oid
target/release/sharc-fxlive FX/core1_sram.bin --core 1 --passes 17000 --in-wav oid/in.wav \
  --script-flat oid/identity.script --wav direct.wav
target/release/lle_sharc_bus --node fx1=FX/core1_sram.bin,fx1,oid/identity.script \
  --route fx1>host --in-wav fx1:oid/in.wav --ticks 34000 --wav bus.wav && cmp direct.wav bus.wav
target/release/lle_sharc_bench --core fx1=FX/core1_sram.bin,fx1,fx-sys-load-core1.script \
  --core fx2=FX/core2_sram.bin,fx2,fx-sys-load-core2.script
```
