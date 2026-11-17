# Research: does the B3 Leslie run on the FX chip? (not supported by the evidence so far, D135)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** the five DSP images (`tools/ns4_dsp_images.py`; flat SRAM per core from `tools/ns4_dsp_ldr.py --sram`: FX core 1 / core 2, PNO core 1 / core 2, ORG core 1 / core 2), the ARM boot logs `fx_core1.script`, `fx_core2.script`, `pno_core1.script`, `org_core*.script` (`tools/arm_bulk_tables.py capture`), the B3 8-key PNO lane (`sharc-sys --pno --epdma --lane 2:0 --wav`, 8,016 passes, peak 0.347, rms 0.0584), `sharc-fxlive` (D125), `sharc-sys --probe`, and the new `tools/fx_code_presence.py`. Numbers only; no audio or firmware content is printed or committed. Builds on [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [fx-oracle-live](fx-oracle-live.md) (D125), [fx-leslie](fx-leslie.md), [fx-type-map](fx-type-map.md) s8 to s10, [fx-chain-install](fx-chain-install.md), [audio-remaining](audio-remaining.md) row 9.
- **Question:** The Leslie processor `0x1256AD` runs in 0 of 76 passes on the live PNO core 1 lane (D126). Does it, or any other Leslie code path, run on the FX chip, and what drives it (D98 drive words, ring DMA, ARM messages)? If it runs, how does organ B3 plus that Leslie compare with the native `organ_b3` and the hardware clip (sub / mid / brilliance and the 11.02 dB gap)?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a committed tool or a command listed under Reproduce, **EMPIRICAL** = measured on this host by a command that is not committed, **ASSUMED** = reading without a source.

## 1. Short answer

**Not supported by the evidence so far; not proven absent.** What was tested, exactly:

1. **Byte-identical code presence** of the PNO core 1 routine `0x1256AD` in the static FX / ORG / PNO core 2 SRAM images (12-byte pieces, step 2). A copy relinked to other addresses scores below 1.0 only in the pieces holding absolute addresses (patched-operand test), so a relinked copy would still score high. Code that is **loaded at run time** (a later ARM bulk transfer, a DMA from the flash tables, a different image build) is not in the static image and is **not tested**.
2. **Boot-only ARM logs.** The logs are the boot / part-load captures. A **recall of a program with amp type 7 "To Rotary"** and `kLeslieEnable = 1` was **never captured**, so which stages the ARM installs then is **untested**.
3. **A muted, booted FX chain.** With the boot log the chain output is 0 even for a pass-through stage (section 3), so the live run cannot show audio from any stage and is vacuous for the audio question; it shows only whether an installed stage address is executed.

No "organ B3 + FX-chip Leslie" render exists, so the band levels and the 11.02 dB gap are unchanged and nothing is ported to `nord-core`.

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Is the Leslie processor `0x1256AD` byte-identical code in any static FX image? | **No.** Fraction of its 12-byte code pieces (first `0x300` bytes, step 2) found anywhere in the target image: FX core 1 **0.00**, FX core 2 **0.00**, ORG core 1 / core 2 0.00, PNO core 2 0.00. The B3 processor `0x121E2B` scores 0.00 in all of them as well. Both routines are in the PNO core 1 image only. Scope: static images, relinked copies would still score; run-time loaded code not covered | VERIFIED (`tools/fx_code_presence.py`) |
| 2 | Are the Leslie *family* routines in the FX images? | **Yes, as the amp cabinet module.** The PNO core 1 routines `0x1237F8` / `0x1236F8` (cabinet), `0x1235AA` (table LFO), `0x125371` (nonlinear resonant filter) and `0x1250CF` score 1.00 / 1.00 / 1.00 / 0.98 / 1.00 in both FX cores (FX core 1 copies at `0x123391`, `0x123243`, `0x12500A`, `0x124D68`; core 2 copies at `0x122A1D`, `0x1228CF`, `0x124696`, `0x1243F4`). These are the library routines [fx-type-map](fx-type-map.md) s10 measured as the Amp cabinet. The ORG images score 0.00 to 0.02. Because the FX chip carries the cabinet, a cabinet-based rotary ("To Rotary", amp type 7) on the FX chip is **not excluded** | VERIFIED (same tool) |
| 3 | Does the boot-time ARM traffic send the Leslie row to the FX chip? | **Not in the boot logs.** One-off scan (the scan script is not committed): the Leslie row words (rotor speeds `0xB2F25` / `0xB2F26`, routine pointer `0xB2F29 = 0x1237F8`, drive words `0xB2F2D` / `0xB2F31` of D98) occur only in `pno_core1.script` (10 words in `0xB2F20..0xB2F40`); `fx_core1.script` and `fx_core2.script` hold 0 words there, and no message of any FX or ORG script carries `0x1256AD`, `0x1237F8`, `0x1236F8`, `0x1235AA` or `0x125371` (`pno_core1.script` carries `0x1237F8` x2, `0x1236F8` x1, `0x1235AA` x1, `0x125371` x1, `0x1250CF` x2, never `0x1256AD`). The FX scripts install only FX-address routines (core 1: `0x1231CB` x5, `0x124D68` x2, `0x124E0B` x3, `0x12500A` x5; core 2: `0x1228CF` x5, `0x122A1D` x2, `0x122B1D` x3, `0x1243F4` x2, `0x124497` x3). Boot logs only; recall traffic not captured | EMPIRICAL (scan not committed) |
| 4 | Does any Leslie-family stage run on the FX chip with the B3 lane as input, booted state? | **No stage runs, but this is vacuous for audio.** `sharc-fxlive` with the FX boot log (master word `0xB6518 = 1.0`), 3,000 lead passes and the B3 lane as input (input L peak 0.1727, rms 0.0165): "not reached in a pass" for core 1 `0x1231CB`, `0x124D68`, `0x124E0B`, `0x12500A`, `0x123243`, `0x123391` and core 2 `0x1231CB`, `0x124D68`, `0x124E0B`, `0x12500A`, `0x1228CF`, `0x122A1D`, `0x122B1D`, `0x1243F4`, `0x124497`; `0x1256AD` is not reached on either core. The output is exactly 0: the boot-log state is the default program with the effects off. The booted chain has no stage installed from a Leslie or rotary recall, so "not reached" says nothing about a recalled B3 program | VERIFIED (`--regs-at`), vacuous for audio |
| 5 | Control: does an installed stage get reached and pass audio? | **Reached: yes. Audio: only without the boot log.** Stage 0 set to the pass-through `0x1231CB` with the B3 lane in: reached with and without the boot log. Without the boot log the output L rms is 0.003081 against input 0.016514 = **0.1866 x**, the D125 transfer. With the boot log the output is again exactly 0 (a mute somewhere in the booted chain that was not located), so the booted chain cannot be used to listen to a stage | VERIFIED (run) |
| 6 | Self-presence control: does the stage-install path find a known FX stage with its module state? | **Not achieved.** Installing the FX core 1 cabinet copy `0x123391` or `0x12500A` into stage 0 reaches it but the pass does not return to the poll loop (1,270 and 1,305 steps, with and without the boot log): the cabinet needs the module state block that only a recall installs. Installing `0x1256AD` into stage 0 likewise does not return (4,906 steps booted, 5,381 clean): the address holds unrelated FX code, not the Leslie. So the run cannot place a Leslie stage in a working chain, and the "reached" answers of row 4 cannot distinguish a missing Leslie from a chain that was never given one | VERIFIED (run) |
| 7 | Does the PNO core 1 pass call `0x1256AD` after the full B3 load and 8 keys? | No: `sharc-sys --probe 0x1256AD` executes it in **0 of 316** passes (D126: 0 of 76) | VERIFIED |

Consequences for the metric: the D126 lane against `organ_b3` stays the only measured pair (sub +27.54 dB, mid +1.04, brilliance +15.41, gap 11.02 dB). There is no FX-chip Leslie render to compare, and the native `organ_b3` render is untouched.

## 2. Method

1. **Code presence.** `tools/fx_code_presence.py REF PC[:name].. --target IMG..` cuts `0x300` bytes at `2 * pc` of the reference image into 12-byte pieces (step 2 bytes) and reports the fraction found anywhere in each target image. A copy linked at another address scores about 1.0 (only pieces that contain an absolute address break), absent code about 0.0. The 1.00 of `0x1237F8` in both FX cores is the positive control for the 0.00 of `0x1256AD`. Unit tests: `tools/tests/test_fx_code_presence.py` (copy at another address, patched operand, absent code).
2. **ARM side.** One-off script scan of the boot logs: final value per DM word and the routine values installed. Not committed, so EMPIRICAL.
3. **Live FX chip.** `sharc-fxlive IMG --core N --passes 3300 --lead 3000 --msg 0xB6518=f1.0 --script-flat fx_coreN.script --in-wav b3lane.wav --regs-at PC` per routine; `--regs-at` runs one more pass and reports whether the PC executed. Controls: stage 0 install record `--msg 0xB62CE=FN --msg 0xB62CF=0xC0458 --msg 0xB62D0=0xB62EE --msg 0xB62D2=0 --msg 0xB62D3=f0.5` with and without the boot log.
4. **Lane.** `sharc-sys PNO --pno --epdma --passes 8000 --warmup 16 --script-flat pno_core1.script --script-flat org-system-load-pno1.script --script-at 16:org-system-key8-pno1.script --lane 2:0 --wav b3lane.wav --rate 43402`.

## 3. What this leaves

- `0x1256AD` exists only in the PNO core 1 image and is named only by the RAM-built channel table `0xC05450C8` (row 6, [fx-leslie](fx-leslie.md)); nothing in the captured ARM logs links it to a chain. The leading reading is that the open question is **the ARM install path on the PNO chip** (which part command or recall phase writes `0x1256AD` into `0xC2F48..`), but the FX chip is **not ruled out**.
- **Untested, could change the answer:** (a) code loaded into the FX cores after boot; (b) an ARM recall of a program with amp type 7 (To Rotary) and `kLeslieEnable = 1` ([fx-type-map](fx-type-map.md) says its words are not in the drive-stage context); (c) a booted-and-unmuted FX chain with a cabinet stage and its state block. The FX chip carries the cabinet, so (b) is the most plausible way for a rotary to appear there.
- The hardware clip `organ_b3` is the final mix after the FX chip, the lane is the dry DSP output (D120). With the FX chain at its default recall silent in the oracle, the FX chip is not what turns the dry lane into the clip in the current oracle state.

## Implications

- Do not wire a Leslie into `nord-core` from this evidence: no FX-chip Leslie processor was found in the static images and no Leslie row traffic in the boot logs, but no render exists either way.
- The B3 presence gap (mid +1.04, brilliance +15.41, gap 11.02 dB on the lane) stays an organ-side / ARM-install question ([audio-remaining](audio-remaining.md) row 9).

## Reproduce

```bash
python tools/fx_code_presence.py PNO/core1_sram.bin 0x1256AD:leslie 0x1237F8:cabB 0x125371:nl 0x1235AA:lfo 0x121E2B:b3proc \
    --target FX/core1_sram.bin --target FX/core2_sram.bin --target ORG/core1_sram.bin
# row 4: booted chain, "not reached" and output 0
target/release/sharc-fxlive FX/core1_sram.bin --core 1 --passes 3300 --lead 3000 --msg 0xB6518=f1.0 \
    --script-flat fx_core1.script --in-wav b3lane.wav --regs-at 0x1231cb
# rows 5 and 6: install a stage in stage 0 (drop --script-flat for the clean-state control)
target/release/sharc-fxlive FX/core1_sram.bin --core 1 --passes 3300 --lead 3000 --msg 0xB6518=f1.0 \
    --msg 0xB62CE=0x1231CB --msg 0xB62CF=0xC0458 --msg 0xB62D0=0xB62EE --msg 0xB62D2=0 --msg 0xB62D3=f0.5 \
    --in-wav b3lane.wav --regs-at 0x1231cb
# row 7
target/release/sharc-sys PNO/core1_sram.bin --pno --epdma --passes 300 --script-flat pno_core1.script \
    --script-flat org-system-load-pno1.script --script-at 16:org-system-key8-pno1.script --probe 0x1256AD
```

## Follow-ups

1. Find the ARM call that writes the module install for the organ part's Leslie (row 6 of `0xC05450C8`, `0x1256AD`) into a PNO core 1 chain head: run the part activation of [arm-part-activation](arm-part-activation.md) with the DSP read-back stub `0xC0066E64` answered from the oracle (its follow-up 1), then diff the PNO core 1 message log against `org-system-load-pno1.script`.
2. Capture an ARM recall of a program with amp type 7 (To Rotary) and `kLeslieEnable = 1` and scan the FX scripts for new stage installs; replay it through `sharc-fxlive` with the module state block so the cabinet stage returns (row 6) and locate the mute of row 5.
3. Commit the boot-log scan of row 3 as a tool so the tag can move from EMPIRICAL to VERIFIED.
4. If a message ever installs `0x1256AD` on PNO core 1, re-run `sharc-sys --probe 0x1256AD` and `tools/dsp_lane_vs_hw.py --segment organ_b3`; port only with VERIFIED provenance.
