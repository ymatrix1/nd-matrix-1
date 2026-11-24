# Arpeggiator gate floats `DM 0xC1396/7`, `0xC13E6/7`, `0xC1436/7`: who reads them and how the gate is applied

- **Checked:** 2026-11-23
- **Sources (local, never committed):** `target/DSP/dsp_image_2_ORG.bin` and `dsp_image_1_PNO.bin` unpacked with `python tools/ns4_dsp_ldr.py IMG --sram DIR`; the ARM-to-DSP capture `dg_gate57.txt.gz` of A:17 Dual Gates (chord held, `tools/arm_arp_events.py` harness, 120 BPM; load/idle/hold rows, derived words only); `sharc-org --rwatch / --watch / --trace`, `sharc-sys --tap`, `sharc-disasm` (read locally, no disassembly committed: D20, D128). Builds on [arp-gate-flag-trace](arp-gate-flag-trace.md), [org-dsp-map](org-dsp-map.md) s3, [org-stage-map](org-stage-map.md).
- **Question:** The ARM writes float 1.0 / 0.0 gate levels to six DSP words in Gate arp mode (A:17: open 84 ms of a 125 ms step) and sends the chord once. The channel 0 voice record (`IN0 = 0xC1F96 - 13`) of an ORG core 2 replay is unchanged by those writes. Which SHARC core and code reads the words, how is the gate applied (multiply, retrigger, smoothing), and at what point of the chain?

Tags follow `.agents/rules/provenance-in-code.md`. Reproduce commands are at the end.

## 1. Answer

**The reader is the per-list-node preamble of the ORG core 2 pass (and the same program on PNO core 2). It runs a one-pole smoother on the gate float and the layer mix multiplies the summed slot bus by the smoothed value just before the bus is written to the pass output buffers.** It is an amplitude multiply with a symmetric ~9.2 ms one-pole edge, not an envelope retrigger and not a hard step.

| # | Finding | Tag |
|---|---|---|
| 1 | Reader pcs: ORG core 2 `0x125EFD`, `0x125FA3`, `0x126049` (loads of the pair `DM(0xC1396/7)`, `0xC13E6/7`, `0xC1436/7`, 2 loads each, one per pass); PNO core 2 `0x125F02` (same pair, 6 loads in 6 passes); ORG core 1 `0x125F18`, `0x125FBE`, `0x126064` (Core1 start of org-dsp-map s2, 4 passes, same `0xA6` spacing). Spacing `0xA6` words = one preamble block per list node (I0 = record `0xC13A6 + 0x28 k`, k = 0, 2, 4; the gate words sit at `I0 - 16` and `I0 - 15`: the two SIMD lanes). | VERIFIED (`sharc-org --rwatch`, 3 windows) |
| 2 | The preamble stores one-pole state into `I0 - 10` and `I0 - 8` (pairs `0xC139C..F`, `0xC13EC..F`, `0xC143C..F`; the two are equal in every pass checked). Stores at `0x125F08..0x125F1B`. | VERIFIED (`--watch`) |
| 3 | Law: `g' = g + c (G - g)` per pass with `G` the gate float, `c = 0.005` (`0x3BA3D70A` in `I0 - 12`) and `1 - c = 0.995` (`0x3F7EB851` in `I0 - 14`), both coefficient words are initialised by the ARM recall, not by the gate. Fit of 3 nodes x 2 edges: `c = 0.005000`, max one-step deviation 4e-8 (fall) and 1.1e-7 (rise). Time constant `-1 / ln(1 - c) = 199.5` passes = **9.19 ms**, 10 to 90 % in 438 passes = **20.2 ms**, largest one-pass change 0.5 % of the step (no hard edge). Rise and fall are symmetric. | VERIFIED (`tools/gate_edge_probe.py`, unit-tested fit) ; pass period 2 frames at 43,402.34 Hz ASSUMED (D121) |
| 4 | Consumer: `R8 = DM(0xC139E)` (`0xC13C6`, `0xC13EE`, ... for k = 1, 2, ...) at pc `0x1285B8` (node 0; the multiply of node k is at `0x1285D2 + 0x24 k`, six nodes k = 0..5). The block loads four stride-2 words from the slot buses (`I3 = 0xB135C + 8 s`, M2 = 2, SIMD pairs), adds the second operand `R9 = DM(0xC1394 + 0x28 k)` times the complementary-register words, multiplies the four sums by `R8` and writes them to the output buffers (`I2 = DM(0xB2EF5)`, `I4 = DM(0xB2EF6)`, here `0xB0000` / `0xB0028`). So the gain acts on the **layer slot bus sum, after every voice stage has accumulated into it and before the output buffer**. | VERIFIED (pcs, registers by `--trace`); data flow of the two operands ASSUMED (compute decode of the SIMD complementary registers read from the listing, not checked by a unit test) |
| 5 | The channel strip stage records (`IN0`), the envelope taps and every word of `0xB0000..0xB2FFF` and `0xC0000..0xC2FFF` are **bit-identical** for gate 1.0 against 0.0 over 300 passes of a held chord (0 of 12,288 + 12,288 words differ except the gate words and `I0 - 10 / - 8`). The gate therefore does not reach the voice or the strip; it multiplies only the bus read at `0x1285B8..`. | VERIFIED (`sharc-sys --tap` of both regions, two runs, 0 differing words) |
| 6 | In this replay the slot bus words at the multiply (`R0..R3`) are **0** for the synth chord: the slot accumulators `0xB135C + 8 s` are cleared at pass start (`0x125E71`, 48 stores) and written by the stage code at `0x127D3A..`, yet trace at the multiply shows 0 in all six nodes up to pass 700. The audio of this chord (`IN0` non-zero, `0xB0100..` pass-buffer words non-zero) does **not** pass through the gated bus. Detail: 12 stage store sites write slot 0 (`0x127D3A`, `0x127E52`, `0x127F6A`, `0x128082`, `0x12810E`, ... `0x12856E`, `I3 = 0xB135C`); the first two see non-zero voice samples in `R0` (e.g. 0.17, -0.12), from `0x128082` on `R0` is 0 (inactive channels), and the value loaded at the multiply (`R0..R3` at `0x1285D2`) is 0 in **all 700 of 700 passes** (key-on at pass 300). How the stage chain leaves the slot bus empty (overwrite by the later stages, or the stages target another buffer through the record pointers) was not resolved. Three stages (`0x127D3A`, `0x127E52`, `0x127F6A`: the three chord notes) see non-zero voice samples in `R0`; the stored register `R14` is 0 at the first and fourth store of those stages and of the later ones, but the stage code is a software pipeline (read-ahead `I2 = I3 + 2`, accumulate `F14 = F11 + F14`, one stage per voice), so a zero `R14` at a fixed pc is not evidence of an overwrite either way. New `sharc-org --trace-dm 0xB135C:8` (appends SRAM words to each `--trace` line, no watch side effect) shows the mid-pass bus: it is **0 on entry to every stage and at the multiply** (6 sites checked, pass 419), so stage 1 (`0x127D25..0x127D80`) itself writes zeros. Its registers: the loop is an interpolating reader (kernel table `DM(0xC08E1..)`, 1,020 of 1,024 words non-zero, index and fraction from `FIX` / `FDEP` of a position word, gain `F8 = 2.0`), and in every traced step the kernel taps `R5` and the interpolation product `R2` and the fraction `F1` are 0, so `F11 = F2 * F8 = 0`. Reading (ASSUMED): these 12 stages are sample-style readers whose source data (ring / stream) is not in the replay (the L2 block of [arm-bulk-tables](arm-bulk-tables.md) is not applied), so the bus is empty here; the A:17 synth chord goes through the strip stages (`IN0`) and not through this bus. Whether the real synth path reaches the gated bus stays open. | VERIFIED (`--trace` at `0x1285D2 + 0x24 k`, passes 410 / 700; `--watch` store sites; zero count over 700 passes) ; the overwrite reading ASSUMED |

## 2. What this means for the synth layer

The multiplier exists and is wired for three layer nodes in ORG core 2 and PNO core 2, but the A:17 synth voice is not on those buses in the replay. Candidates, none tested: (a) the synth layer is summed into the gated bus only by a stage state the capture does not reach (only ORG core 2 and PNO core 2 words were replayed, no SY / FX chip); (b) the gate is applied to organ / piano layer buses and the Gate-mode synth layers are gated by another path (SY1 / SY2 image: the ARM sends no gate word there); (c) the replay lacks a word that routes the synth stages into slot buses. The ARM side is unambiguous (one chord, gate floats only), so for a synth layer the audible gate must act through (a) or (c) or it is not applied. **Open: OQ43.**

## 3. Edge shape (for the native engine)

From the `record - 8` multiplier, which is exactly the gain applied by the multiply (the bus is linear in it):

| Quantity | Value | Tag |
|---|---|---|
| Edge | first-order, `g += 0.005 (G - g)` per pass, rise = fall | VERIFIED (3 nodes, both edges) |
| Time constant | 199.5 passes = 9.19 ms (pass = 46.08 us) | VERIFIED in passes, ms ASSUMED (D121) |
| 10 to 90 % | 438 passes = 20.2 ms | same |
| Hard step? | no: 0.5 % of the step per pass | VERIFIED |
| Gate level | 1.0 / 0.0 floats only; no ENV (RANGE/ENV) word reaches the DSP (stream identical for 0, 57, 127) | VERIFIED (ARM trace, previous PR) |

The ENV smoothing of `tick_gate` (`tau = 0.25 * step * (1 - env) + 0.5 ms`) is therefore **not** the firmware's: the hardware edge is a fixed 9.2 ms one-pole for every ENV value.

## Implications

- Native `ArpMode::Gate` (`crates/nord-core/src/synth/perf.rs`, `tick_gate`) should: keep the layer playing (no retrigger, as now), multiply the layer output by `g` with `g += (1 - exp(-1/τ)) (G - g)`, `τ = 9.19 ms` fixed (c = 0.005 per 46.08 us pass; at 48 kHz `c_s = 1 - 0.995^(pass_len_in_samples)` or the exact `1 - exp(-dt / 9.19 ms)`), independent of `gate_env`, with `G = 1` for the first 2/3 of a step and `0` after. Not changed here.
- The spec bullet "ENV smoothing ASSUMED" is replaced by this measurement (updated in `docs/specs/synth-engine.md`).
- Before porting to the synth layer, answer OQ43: whether the synth layer audio is on the gated bus on the real chain.

## Follow-ups

1. OQ43: give the slot-bus readers their source data (ring / L2 block, or a SAMPLE layer program) and use `sharc-org --trace-dm` to confirm the bus becomes non-zero and the output buffer after the multiply follows the gain fit; or find the synth path's own gate consumer if it is not this bus.
2. ORG core 1 is confirmed as a reader (row 1); its smoothing and multiply were not measured, only assumed equal (same program).
3. Third pair per node: `R9 = DM(0xC1394 + 0x28 k)` multiplies a second operand into the sum (0 in the image); find its ARM writer (cross-layer send?).
4. Hardware clip of an arp gate edge to test the 9.2 ms reading.

## Reproduce

```bash
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram target/gate/ORG        # core2_sram.bin
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_1_PNO.bin --sram target/gate/PNO
# 1. readers (replay of the capture: load rows at pass 0, the chord and gate words later; message script `PASS DM VALUE`)
sharc-org target/gate/ORG/core2_sram.bin --dev 0x282c0380=1 --dev 0x3108d008=5 --pre 0x3e021=1 --passes 20 \
    --msg-base 0 --msg-script org.script --rwatch 0xC1396:2:100       # also 0xC13E6:2, 0xC1436:2
sharc-org target/gate/ORG/core1_sram.bin --dev 0x3108d008=5 --dev 0x28ac0380=1 --dev 0x3108c06c=0x400 --dev 0x3108c00c=4 --flag 1=1 --boot 3000000 --trigger 0xB00E1 --idle-pc 0x125DF3 --passes 4 --rwatch 0xC1396:2:100
sharc-org target/gate/PNO/core2_sram.bin --dev 0x282c0380=1 --dev 0x3108d008=5 --pre 0x3e021=1 --passes 6 --rwatch 0xC1396:2:100
# 2. edge shape (taps record-8 of nodes 0, 2, 4 after every pass; gate 1 -> 0 -> 1 at passes 2000, 4000)
cargo build --release -p nord-sharc --bin sharc-sys
uv run python tools/gate_edge_probe.py CAPTURE.txt.gz --image target/gate/ORG/core2_sram.bin --sharc-sys target/release/sharc-sys
uv run pytest tools/tests/test_gate_edge_probe.py
# 3. gate does not touch the voice or the pass buffers: two sharc-sys runs (gate word 0.0 / 1.0 at pass 400), --tap 0xB0000:0x3000 --tap 0xC1F89:8, compare words
```

Result of step 2 (identical for the three nodes): fall and rise `c_per_pass 0.005`, `tau_ms 9.193`, `t10_90_ms 20.183`, `max_step_fraction 0.00497`.
