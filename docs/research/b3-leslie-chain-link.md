# Research: linking `0x1256AD` into the PNO core 1 chain by hand, and what feeds the chain (D171)

- **Checked:** 2026-11-20
- **Sources (local, never committed):** PNO core 1 SRAM image `target/PNO/core1_sram.bin`, ARM boot log `target/bulk/pno_core1.script`, committed streams `docs/research/data/org-system-load-pno1.script` and `org-system-key8-pno1.script`, tools `sharc-sys` (`--pno --epdma --msg --probe --dump --lane --wav`), `sharc-ring` (`--trace`), `sharc-disasm`. Numbers and addresses only; no firmware, sample or audio content is printed or committed. Builds on [leslie-paths](leslie-paths.md) (D155), [leslie-gate](leslie-gate.md) (D148), [leslie-locate](leslie-locate.md) (D139), [arm-dsp-live-link](arm-dsp-live-link.md) (D138), [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [b3-leslie-fx-chip](b3-leslie-fx-chip.md) (D135), [organ-preamp-search](organ-preamp-search.md) s6, [audio-remaining](audio-remaining.md) row 9.
- **Question:** The ARM never installs `0x1256AD` (0 of 2,441 live passes, D138 / D155). If the stage is linked into a chain head by hand, does it run, what does it read, and does the B3 lane output (`--lane 2:0`) move? That tells whether a missing install is the whole B3 mid / brilliance gap, or whether something else is also missing.

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command under Reproduce, **EMPIRICAL** = measured on this host, **ASSUMED** = reading without a source.

## 1. Short answer

**Negative, narrowed. The ARM install is still not found, and linking the stage by hand does not change the B3 lane.** The stage runs and returns when linked, but the chain it sits in has no organ feed in the oracle, and its output is not on the lane path. So even a found install would not by itself move mid +3 dB / brilliance +5 dB; the missing piece is a producer of the chain input, not only the head write. Nothing is ported; the native Leslie (`--leslie firmware`, D63, D94) is unchanged.

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Element layout the chain loop `0x1213E9` expects | Head `DM(0xC2F48 / 0xC2F82 / 0xC2FBC / 0xC2FD6)` holds the address `E` of an element; `DM(E-32)` routine (`I8`, called with `CALL (M8, I8)`), `DM(E-31)` input pair pointer (`I0`; L at `+0`, R at `+2`), `DM(E-30)` state pointer (passed in `I1`), `DM(E-29)` next element, `DM(E-28 .. E-25)` four words (the null stub holds `-0.01, -0.5, -0.5, 0.5`). The null stub at `E = 0xC2E4A` links to itself, which is why the unrolled loop ends on it | VERIFIED (`sharc-disasm 0x2427d2..` listing, `--dump 0xC2E00:0x240`) |
| 2 | Does a hand-linked Leslie element run? | **Yes.** Words written at pass 40 (`--msg 40:...`): `DM(0xC2E90..0xC2E97) = 0x1256AD, 0xC2E06, 0xB2F2A, 0xC2E4A, 0xbc23d70a, 0xbf000000, 0xbf000000, 0x3f000000` and `DM(0xC2F48) = 0xC2EB0`. `--probe 0x1256AD`: executed in **276 of 316 passes** (0 of 316 without the injection, same run). Every pass returns to the poll loop (`all idle: true`). `DM(0xB4CCC) = 0xC2E1A`: the stage reads the chain scratch copy, as D94 s6.1 read from the listing | VERIFIED (run) |
| 3 | Does it change the B3 lane? | **No.** `--lane 2:0`, 2,016 passes, 8 keys held: peak `0.239434`, rms `0.052241` with and without the injection, and the two `.wav` files are **byte identical** (`cmp`) | VERIFIED (run) |
| 4 | What is in the chain input? | `DM(0xC2E06..0xC2E09)` is **0** after 300 passes with and without the injection; in pass 60 the only writers of `0xC2E06 / 0xC2E08` are the chain loop's own element-output stores (`pc 0x121435`, `0x121482`, ...). No slot routine (`0x121E2B` B3 processor and the others) writes the chain pair. So the chain runs on silence | VERIFIED (`sharc-ring --trace 0xC2E06:4 --from 60 --to 61`) |
| 5 | The other feeds of the stage | `DM(0xB0078..)` and `DM(0xB008C..)` (the two DMA input lanes, [organ-preamp-search](organ-preamp-search.md) s6.1) are 0, and the aux pair word `DM(0xB2F3C)` is 0 (the gain is 0 in the image). No PNO instruction stores to the lanes. The Leslie therefore also runs on zeros here | VERIFIED (`--dump`) |
| 6 | Where does the chain output go? | The chain loop writes the element output back to `0xC2E06` and a later record routine (`0x2436D0`, pc `0x121B68`) moves a pair `DM(0xC2E0E)` into a ring pointed to by `DM(E-24)`; this routine did not run in pass 60 of the lane (early `IF GE RTS`). Nothing in the lane path reads `0xC2E06` (the only consumer of the address as an immediate is the head call at `0x243694`). Reading: the chain is an in-place effect bus that hands over to another chip by DMA, **ASSUMED**, not traced | VERIFIED (listing, trace) for the writers and readers; ASSUMED for the hand-over |

## 2. What this says about the open install

1. **Mechanically the install is eight DM words** (one element and one head), and the stage tolerates being linked: it returns on every pass. This is the first positive control that a PNO core 1 Leslie element can run in the oracle, and it removes the "needs module state that only a recall installs" worry of D135 row 6 for this chip (the state block `0xB2F2A` is the one the boot log fills).
2. **A head write is not enough for audio.** The lane pair `2:0` is the dry organ lane (D120); the chain pair has no writer other than the chain. In hardware the B3 Leslie therefore has a feed this oracle does not produce. The candidates are the two DMA input lanes `0xB0078 / 0xB008C` (filled by a DMA, [organ-preamp-search](organ-preamp-search.md) s6.1, never read in the lane run) or an element before the Leslie that copies the organ into the chain pair. Either needs a DMA descriptor or a head that the ARM stream of D138 does not contain.
3. **Combined with D138 / D155** (no head word, no `0x1256AD` word, no L2 store, no image load after the boot in 7 recalls and 2,966 edit phases each, live PNO 1: 0 of 2,441 passes), the two missing items (head write and chain feed) point to one mechanism outside the ARM word stream, for example a DSP side transfer set up by the boot image, not a part command. This is ASSUMED; it is consistent with H1 of [leslie-paths](leslie-paths.md) s5.

## 3. Search state after this note

| Lead | State |
|---|---|
| Part command consumer `[part + 0xC8]` | Not run. D139 / [arm-part-activation](arm-part-activation.md) s1 already show no polled queue at `[part + 0xC8]`; the activation words of the default program are sent in `settle 2`. Needs the oracle-answered read-back stub `0xC0066E64`; not attempted here (no new harness code) |
| Rotary-on paths (UART speed switch, organ on / off) | Not run (H3 of D155) |
| 5,910 out-of-window ARM stores | Not run; D155 row 2 reports 0 L2 / code stores in the 7 recalls, so the out-of-window stores of D138 are boot bulk stores, ASSUMED |
| Run-time loaded code | Closed for the ARM side (D155 row 2); DSP-side DMA from flash not modelled (D141) |
| Other cores | PNO 2 and ORG 2 hit the same pc but it is the filter stage `0x12566E` (D155 correction 1); FX cores do not carry the routine (D135) |
| Chain feed (new) | Open: who writes `0xC2E06` or fills `0xB0078 / 0xB008C` on hardware |

## Implications

- Do not look for the install alone: a head write without a feed changes nothing on the lane. The next experiment needs a producer (the ORG / SY hand-over or the DMA descriptors) in the live link, not another ARM edit.
- The B3 mid +3 dB / brilliance +5 dB gap stays with the ring DMA history and the unmodelled hand-over ([organ-preamp-search](organ-preamp-search.md) s6.2); no opt-in port is added, `--leslie firmware` stays as measured in D94 / D148.
- No default, golden, `render_hash` or `nord-core` change.

## Follow-ups

1. Trace the hand-over: find what reads `0xC2E06` after the chain (the ring behind `DM(E-24)`, routine `0x2436D0`) and the DMA descriptor that moves it; run it with the organ lane copied into `0xC2E06` to see whether the lane-to-chain mix is `proc * wet + lane * DM(0xB00EE)`.
2. Fill `0xB0078 / 0xB008C` with the B3 dry lane in the oracle (a two-lane DMA stand-in) and re-run the hand link of row 2; compare the stage output against the clip bands with `tools/dsp_lane_vs_hw.py`.
3. H2 / H3 of [leslie-paths](leslie-paths.md): part command consumer and the UART speed switch in the live link.

## Reproduce

```bash
M=<main checkout with target/ and .reference/>; S=docs/research/data
cargo build --release -p nord-sharc --bin sharc-sys --bin sharc-ring --bin sharc-disasm
T=target/release
ARGS="$M/target/PNO/core1_sram.bin --pno --epdma --passes 300 --warmup 16 \
  --script-flat $M/target/bulk/pno_core1.script --script-flat $S/org-system-load-pno1.script \
  --script-at 16:$S/org-system-key8-pno1.script"
MSG=""
for kv in 0xC2E90=0x1256AD 0xC2E91=0xC2E06 0xC2E92=0xB2F2A 0xC2E93=0xC2E4A 0xC2E94=0xbc23d70a \
          0xC2E95=0xbf000000 0xC2E96=0xbf000000 0xC2E97=0x3f000000 0xC2F48=0xC2EB0; do MSG="$MSG --msg 40:$kv"; done
$T/sharc-sys $ARGS --probe 0x1256AD                      # executed in 0 of 316 passes
$T/sharc-sys $ARGS $MSG --probe 0x1256AD --dump 0xB4CCC:2 # executed in 276 of 316 passes, DM(0xb4ccc..): c2e1a c2e1a
# lane comparison (2,016 passes): peak 0.239434, rms 0.052241 in both, cmp finds the wavs identical
$T/sharc-sys ${ARGS/--passes 300/--passes 2000} --lane 2:0 --rate 43402 --wav base.wav
$T/sharc-sys ${ARGS/--passes 300/--passes 2000} $MSG --lane 2:0 --rate 43402 --wav inj.wav && cmp base.wav inj.wav
# chain pair writers and element layout
$T/sharc-ring $M/target/PNO/core1_sram.bin --script-flat $M/target/bulk/pno_core1.script \
  --script-flat $S/org-system-load-pno1.script --script-at 16:$S/org-system-key8-pno1.script \
  --trace 0xC2E06:4 --from 60 --to 61
$T/sharc-disasm $M/target/PNO/core1_sram.bin 0x2427d2 0x242830
```
