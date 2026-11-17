# Reverb type 0 and CHOR in the live FX oracle against the native ports: causes and fix

- **Checked:** 2026-11-17
- **Sources (local, never committed):** the FX Core 1 flat SRAM image (`tools/ns4_dsp_ldr.py --sram` of the v1.66 FX image), the oracle `sharc-fxlive` (`crates/nord-sharc`, [fx-oracle-live](fx-oracle-live.md), D125), the native renderer `fx_live_native` (`crates/nord-tools`), `tools/fx_oracle_live.py`, `sharc-disasm` listings of the pass routine. Only numbers derived from runs are quoted; no firmware, sample or audio content is printed or committed.
- **Question:** why do the live oracle responses of reverb type 0 (routine `0x124AF2`) and CHOR (`0x123798`) differ from `FwReverb` and `FirmwareFx::mod2`, and which side is wrong?

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Reverb type 0 | **No port error.** The D125 mismatch was the harness: the dispatcher called the installed stage **35 times per pass**, not once. Reverb onset 38 passes = 1352 calls / 35. With one call per pass the oracle equals `FwReverb`: lag 0, gain 0.1866, null depth **-137.0 dB** (impulse), **-130.2 dB** (noise), 1.0 s each | EMPIRICAL (local-only: `tools/fx_oracle_live.py reverb0`; the port was already VERIFIED against golden vectors) |
| CHOR type 4 | **Port bug, fixed.** After the harness fix the right channel matched, the left wet tap was missing in the port (oracle: wet at 32/33 frames at phase 0, 77/78 after 12000 frames; port: none). Cause: the port wrapped the register pre-modify store `DM(M6, I6)` (`I6 + 0x202`, `L6 = 0x200`) into the first ring, so the guard copy was never written (the error D114 removed from ENS and SPIN). Fixed with `Pe::plain`. Null depth before / after: -18.0 / **-125.3 dB** (noise), -19.7 / **-153.6 dB** (burst), -18.9 / **-126.2 dB** (impulse); rate 0, 127 and amount 0, 127 variants: -119 to -156 dB | EMPIRICAL (local-only live check) |
| CHOR variation (type 5) | Same bug, same fix. Null depth -26.2 / **-89.5 dB** (noise), -92.8 dB (impulse), -91.9 dB (burst). **Only rate 64 / amount 64 is compared** (the oracle script loads one fixed context for `chor5`; native runs at other values do not match it, -23 dB, by construction). Measured residual: rms 1e-6 to 3e-6, max 1.7e-5 on a peak of 0.054 (below one 16-bit LSB, 3.05e-5), uncorrelated with the input at lags 0..128, growing with signal level. The cause (rounding of the port's f32 against the routine's 40-bit registers, or one small context word) is a **hypothesis, not isolated**; it is not the guard-copy error (that gives -26 dB). Not extended to other contexts | EMPIRICAL |
| State-by-state check | An 80-call comparison from the live oracle state (ring, context, input block) gave max 9.2e-7 and located the one wrong ring word (`0x2E2`). That probe was a scratch program and is **not committed**; it is not used as evidence here. The committed check is `tools/fx_oracle_live.py` (needs the local FX image) | observed, not reproducible from the repo |
| Latency | The port stage adds 4 frames (it collects a block); the oracle chain has none. The comparison script finds lag -4 for CHOR. A port property, not a defect of the routine | observed |
| Provenance | `FwReverb` stays VERIFIED by golden vectors; the live chain agrees. CHOR: the old VERIFIED claim (direct-call scenarios) is **withdrawn**: it missed this bug. The header of `fw_chorus.rs` now says EMPIRICAL with the local-only live check. The regenerated goldens come from the corrected Python twin and pin the port to the twin only, not to hardware or the oracle | - |

## 2. Why the stage ran 35 times

The Core 1 pass routine is unrolled: 35 `CALL (M8, I8)` sites in 10 groups (5, 2, 5, 2, ...). Each group starts with `I1 = DM(header)` (headers at `DM(0xB63EC, 0xB6416, 0xB6428, 0xB6452, 0xB6464, 0xB648E, 0xB64A0, 0xB64CA, 0xB64DC, 0xB6506)`), every later site takes `I1 = DM(state - 0x1D)`-style links. In the image all headers and the head record's link (`DM(0xB62D1)`) point at the head record `0xB62EE`, the record D125 installed the stage in. `sharc-fxlive --calls-at 0x124AF2` (new) counts 35 entries per pass, all with state `0xB27E8`; with only the head link fixed 10; with the link and the nine later headers pointed at a copy of the null stub record (8 words at free `DM(0xB6600)`, new `null_chain()` in `fx_oracle_live.py`) exactly 1. The identity stage is unaffected (0.1866 either way: the stage output replaces the bus, it is not added), which is why the D125 identity check passed.

So the three D125 candidates (scratch protocol, instance layout, CHOR context) were not the cause: the scratch holds the scaled input (the probe pass had no signal), and the CHOR context location does not change the result (`0xC13A4` and `0xB2040` give identical numbers).

## 3. The CHOR bug

Ring stores in `0x122E24` (core 1 `0x123798`): `DM(I6, M6) [pre-mod] = R` with `M6 = 0x202` and `L6 = 0x200`, then `DM(I6, M1) = R` (post-modify). PRM fig 6-5: only the post-modify and `MODIFY` update wrap; the pre-modify address is `I + M`. The ports used `dmod` (wrapping) in address positions, so the mirror store hit `I6` again. Ring difference at routine entry (oracle against port): one word, the mirror of the left sample, at ring offset `0x2E2` (0x200 + 0xE2). The left tap reads the mirror region once the read pointer passes the ring end; without the copy it read zeros. Right reads the primary copy, hence matched.

## 4. Defaults and renders before / after

`render_hash` (full engine, synth, organ, piano, FX sweep; `target/release/render_hash`), built from origin/main and from this branch: SHA-256 `46f4dc64fc6de62eac40f1ee8549548d3965eea2d86aa5d59b828ed6566da506` both. Equals the `WHOLE` of `tests/golden_render.rs` (test passes).

`program_sweep` on the factory backup (`--limit 40 --every 10`, 39 programs, local file), old and new binary: the `renders/default` blocks are identical for all 39 programs. The `renders/firmware` block (`--firmware-all`, which turns on `mod2`) differs for 9 of 39 programs (5, 20 to 23, 26, 29, 31, 33): rms changes of 0.1 to 11 percent, peak up to +17 percent (program 20: 0.495 to 0.590, `rms_delta_db` 2.65 to 3.75). So the opt-in `mod2` CHOR path now carries the left wet tap that was missing; the default path is unchanged because `mod2` is opt-in (D22).

## 5. Instances

Two CHOR routines exist in the port, `0x122E24` (kind Soft, type 4) and `0x122F77` (Variation, type 5). Fixed address-position `dmod` sites: 10 in `fw_chorus4.rs`, 8 in `fw_chorus5.rs`; the `Di(..)` register updates keep wrapping. No third CHOR instance and no remaining wrong one is known. A "fifth instance" in the task brief is not something this work found; the generated ports `fw_flang`, `fw_vibe`, `fw_phas1`, `fw_pump`, `fw_rm`, `fw_wah`, `fw_awah`, `fw_spin`, `fw_ens` were searched for `dmod` in address positions (none left, grep only; ENS and SPIN were fixed by D114). The ones not compared live are listed in the follow-ups.

## Implications

- Reverb type 0 and CHOR 4 / 5 (at rate 64, amount 64) match the oracle through the real dispatcher (`tools/fx_oracle_live.py`: reverb0, chor4, chor5). CHOR 5 at other contexts is untested.
- CHOR on the `FirmwareFx::mod2` path (opt-in, D22) changes in the left wet channel (section 4); defaults and `render_hash` are unchanged.

## Follow-ups

1. ENS, FLANG, PHAS, VIBE through the live chain (the null-chain install makes them one call per pass).
2. Feed a PNO lane WAV as FX input; decide whether the 7-layer block needs more than layer 0 (D125 open item).
3. Commit an oracle fixture (derived numbers) for CHOR so CI checks the port against the oracle; make `chor5` load the context for other rates and amounts and isolate the -90 dB residual.
4. A hardware clip for CHOR only after this; OQ21a unchanged.

## Reproduce

```bash
cargo build --release -p nord-sharc -p nord-tools
python tools/ns4_dsp_ldr.py <fx>/dsp_image_0_FX.bin --sram <dir>
python tools/fx_oracle_live.py reverb0 --image <dir>/core1_sram.bin --signal impulse --seconds 1.0
python tools/fx_oracle_live.py chor4   --image <dir>/core1_sram.bin --signal noise   --seconds 1.0
python tools/fx_oracle_live.py chor5   --image <dir>/core1_sram.bin --signal noise   --seconds 1.0
```
