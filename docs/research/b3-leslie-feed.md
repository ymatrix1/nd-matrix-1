# Research: feeding the hand-linked B3 Leslie element from the dry lane (D172)

- **Checked:** 2026-11-20
- **Sources (local, never committed):** PNO core 1 SRAM image `target/PNO/core1_sram.bin`, ARM boot log `target/bulk/pno_core1.script`, committed streams `docs/research/data/org-system-load-pno1.script` and `org-system-key8-pno1.script`, the Part A demo clip (segment `organ_b3`, 1.8 s slice) under `.reference/`; tools `sharc-sys` (new `--feed`, `--wav-chain`, `--feed-tap`; `--ring-dma`, `--msg`, `--lane`), `sharc-disasm`, `tools/dsp_lane_vs_hw.py`. Numbers and addresses only; no firmware, sample or audio content is printed or committed. Builds on [b3-leslie-chain-link](b3-leslie-chain-link.md) (D171), [leslie-paths](leslie-paths.md) (D155), [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [organ-preamp-search](organ-preamp-search.md) s6, [audio-remaining](audio-remaining.md) row 9.
- **Question:** D171 linked `0x1256AD` into chain head `DM(0xC2F48)` by hand, but the lane stayed byte identical because the chain input pair `0xC2E06` was zero. If the oracle supplies that feed (the dry B3 lane copied into `0xC2E06`), does the Leslie output look like the hardware clip, and does the B3 mid +3 dB / brilliance +5 dB gap close?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command under Reproduce, **EMPIRICAL** = measured on this host against the single clip, **ASSUMED** = reading without a source.

## 1. Short answer

**Negative, narrowed. Feeding the chain is mechanically easy and it makes the stage produce output, but with the only parameters the oracle can supply (the element gain words and the Leslie state block are not found on any ARM path) the output is broadband junk that moves every clip band the wrong way. The gap does not close; nothing is ported to `nord-core`.**

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Why D171's hand link could not make sound even with an input | The chain loop (`0x2427D2..`) scales the input pair into the scratch `0xC2E1A` by `g = clip(E-27 + E-28, +-E-25) + E-25` and stores the clipped sum back to `E-27`: `E-27` is a ramped mix position, `E-28` its slope, `E-25` its limit. D171 copied the null stub words (`E-28 = -0.01`, `E-27 = -0.5`, `E-25 = 0.5`), so `g = 0`: the stage was fed silence twice over. The scratch pair `0xC2E1A` stays `0` for the whole run in that case | VERIFIED (listing `sharc-disasm 0x2427d2 0x242830`; scratch tap) |
| 2 | Feed stand-in | `sharc-sys --lane 2:0 --feed 2:0:0xC2E06:p` copies the lane pair of the previous pass into `0xC2E06` before every pass (`p` = `[L0 L1 R0 R1]`, left at `+0`, right at `+2`, as the loop reads `DM(0,I0)` and `DM(2,I0)`; `v` = lane order). One pass of latency. With the null stub chain (no hand link) the chain pair equals the lane pair: rms 0.05222 vs 0.05224, bands 0.0 dB | VERIFIED (run, `feed_run` tests) |
| 3 | Hand link with an open gate (`E-28 = 0`, `E-27 = 0.5`, `E-26 = -0.5`, `E-25 = 0.5`, so `g = 1`) | `0x1256AD` runs in 1,976 of 2,016 passes; the chain pair after the pass has rms **-14.5 dB** re the lane and per band re the lane: sub -29.2, low -20.6, mid -21.5, presence +1.7, brilliance +11.2 | EMPIRICAL (one run, 2,016 passes, 8 keys held) |
| 4 | Against the `organ_b3` clip (`dsp_lane_vs_hw.py --slice-s 1.8`, deltas emu - hw in dB) | lane alone: sub +28.6, low -2.9, mid +1.5, presence +8.1, brilliance +13.7, air -7.8, gap **10.93**. Chain output alone: mid +0.8, presence +28.2, brilliance +43.5, air +68.8, gap **17.79**. Lane + chain: gap **15.77**; 0.5 lane + chain: **17.80**. Layout `v` (verbatim): brilliance +45.5, air +52.4, gap **18.35** | EMPIRICAL |
| 5 | With the ring DMA model (`--ring-dma`) | The chain output is **+39.7 dB** over the clip RMS (unstable), gap **38.3**: the ring contents the stage reads are not a valid history | EMPIRICAL |
| 6 | What the hand link still lacks | (a) the element gain words `E-28..E-25` and the state block `0xB2F2A..` (boot log fills only `0xE359E, 0x1237F8, 0xE31A6`), (b) the delay ring behind `DM(E-24)`, (c) the hand-over of the chain output (D171 row 6, routine `0x2436D0`). None is produced by an ARM word (D138, D155). The output of rows 3 to 5 is therefore a property of the guessed parameters, not of the hardware Leslie | ASSUMED |

## 2. Reading

1. **The feed is not the missing piece.** A plain lane copy is enough to make the stage process audio, so a single missing producer does not explain why the oracle shows no Leslie; the install (head word, element words, state, ring) is still missing as a whole. That agrees with D171 / D155 and it removes "nothing writes `0xC2E06`" as the only blocker.
2. **The oracle cannot measure the hardware Leslie transfer from this chip** until the element words and the ring are known. With the guessed words the result is further from the clip than the dry lane (gap 17.8 vs 10.9), and with the only ring model available it diverges. A clip-closing claim needs either the ARM install or a firmware-derived parameter block; neither exists.
3. **The native Leslie is unchanged.** `--leslie firmware` (D63, D94) already runs the same stage on a surrogate ring and gives presence +1.5 to +2.8 dB; its mid +3.1 .. +3.8 dB and brilliance +4.5 .. +5.8 dB gap stays with the ring / hand-over (organ-preamp-search s6.2), as before.

## Implications

- No default, golden, `render_hash` or `nord-core` change; the harness flags are opt-in and off by default.
- OQ42 narrows from "what feeds the chain pair" to "what installs the element gain words, the state block and the ring behind `DM(E-24)`"; the feed question is answered (a copy of the dry lane, one pass of latency, layout `[L0 L1 R0 R1]`).

## Follow-ups

1. Trace the writer of `E-28..E-25` for the default chain elements (the null stub words come from the image; a Leslie element's words must come from somewhere): search the 38,007 boot messages for stores into `0xC2E90..0xC2FFF` with the Leslie routine as a value, and the ring pointer `DM(E-24)`.
2. Part command consumer `[part + 0xC8]` and the UART speed switch in the live link (H2 / H3 of [leslie-paths](leslie-paths.md)).
3. If a parameter block is ever found, repeat rows 3 to 5 with `tools/dsp_lane_vs_hw.py`; only then consider an opt-in `nord-core` port.

## Reproduce

```bash
M=<main checkout with target/ and .reference/>; S=docs/research/data
cargo build --release -p nord-sharc --bin sharc-sys --bin sharc-disasm
T=target/release
ARGS="$M/target/PNO/core1_sram.bin --pno --epdma --passes 2000 --warmup 16 \
  --script-flat $M/target/bulk/pno_core1.script --script-flat $S/org-system-load-pno1.script \
  --script-at 16:$S/org-system-key8-pno1.script"
# open-gate hand link: routine, input pair, state, next, E-28=0, E-27=0.5, E-26=-0.5, E-25=0.5, head
MSG=""; for kv in 0xC2E90=0x1256AD 0xC2E91=0xC2E06 0xC2E92=0xB2F2A 0xC2E93=0xC2E4A 0xC2E94=0 \
   0xC2E95=0x3f000000 0xC2E96=0xbf000000 0xC2E97=0x3f000000 0xC2F48=0xC2EB0; do MSG="$MSG --msg 40:$kv"; done
$T/sharc-sys $ARGS $MSG --lane 2:0 --rate 43402 --feed 2:0:0xC2E06:p \
  --wav lane.wav --wav-chain chain.wav --feed-tap 0xC2E1A=scratch.wav
python tools/dsp_lane_vs_hw.py --video "$M/.reference/<Part A video>.webm" --workdir work \
  --slice-s 1.8 lane.wav chain.wav
# add --ring-dma for row 5; use 0xC2E94=0xbc23d70a 0xC2E95=0xbf000000 for the D171 null-stub words
```
