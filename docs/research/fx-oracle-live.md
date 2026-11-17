> **Update 2026-11-17 (D133, [fx-reverb0-chor-oracle-diff](fx-reverb0-chor-oracle-diff.md)):** the mismatches of s3 and s4 were the harness (35 stage calls per pass) plus a CHOR port bug; both families now match the oracle. Candidates 1 to 3 of s4 were not the cause.

# FX stages on live input in the SHARC oracle (reverb, chorus) against the native FX models

- **Checked:** 2026-11-16
- **Sources (local, never committed):** the FX Core 1 flat SRAM image (`python tools/ns4_dsp_ldr.py --sram` of the v1.66 DSP image), the `sharc-fxlive` oracle run (`crates/nord-sharc/src/bin/sharc_fxlive.rs`, `runner/fx_run.rs`), the native renderer `fx_live_native` (`crates/nord-tools/src/bin/fx_live_native.rs`), the comparison script `tools/fx_oracle_live.py`. Builds on [dsp-peripherals-audio](dsp-peripherals-audio.md), [fx-dsp-map](fx-dsp-map.md), [fx-chain-install](fx-chain-install.md), [fx-routine-models](fx-routine-models.md), [fx-reverb-leslie](fx-reverb-leslie.md), [fx-reverb-colors](fx-reverb-colors.md), [fx-mod2-validation](fx-mod2-validation.md).
- **Question:** can the FX chip pass run on a live stereo signal in the oracle (the whole chain: input stage, 35 stage calls, master), with one real effect routine installed, so that a reverb and a chorus can be checked against `FwReverb` and `FirmwareFx::mod2` on the same input instead of one routine at a time?

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Live FX pass | `sharc-fxlive IMAGE --core 1\|2` boots an FX core, delivers ARM messages, writes 4 stereo frames per pass into the input block, runs the pass with the `P4I` start of the PNO lane (core 1) and collects the 4 output frames. No peripheral beyond the existing FX `devs` presets is needed; `--census` (D120) lists every access outside SRAM | VERIFIED (run) |
| Chain transfer with a pass-through stage | Stage 0 (`0xB62CE`) set to the pass-through routine `0x1231CB`, master `DM(0xB6518) = 1.0`, 1.5 s burst: the chain output equals **0.1866 x input** at lag 0, null depth **-157.6 dB** after the gain fit (65,100 frames), centroid 721.02 Hz both. The 0.1866 is the Core 1 input-stage master gain already measured in [piano-receiver-scale](piano-receiver-scale.md) | VERIFIED (`tools/fx_oracle_live.py identity`) |
| Fade-in | The master stage fades in over about 2000 passes: an input before pass ~3000 is attenuated. The script prepends 3000 silent passes (`--lead`) | VERIFIED (run, null depth without lead -12.6 dB, with lead -157.6 dB) |
| Reverb type 0 (Spring) in the live chain | The stage installs (routine `0x124AF2`, state `0xB27E8`, the 39 type words, `kRvbAmt` send 1 / dry 0, buffers zeroed) and runs, but the response **does not match** the native `FwReverb`: section 3 | measured, **not reachable as is** |
| Mod 2 CHOR (type 4) in the live chain | Same: installed (`0x123798`, context `0xC13A4`), output does not match `FirmwareFx::mod2`; the first run also emits values up to 1e38 before the signal starts | measured, **not reachable as is** |
| ENS, FLANG, PHAS | Not attempted: the CHOR install is the reference for them and does not reproduce yet | open |
| PNO lane into the FX input | Not wired: the harness feeds the input block `0xC01BC` (section 0, bus 0) from a WAV; a `sharc-sys --lane --wav` capture can be used as that WAV. Whether the 7-layer 60-word block needs more than layer 0 is open | open |

## 2. Method

1. **Boot** the core (`FxCore::boot`, same device presets as `sharc-fx`), then per pass: deliver due messages (`(DM word, value)` pairs, the ARM boot-log format), write the input block, `lean_pass` from the `P4I` interrupt (core 1) or the mailbox (core 2), read the output.
2. **Stage install** (core 1): the record `S - 0x20 .. S - 0x1B` in front of the state block holds `fn` (routine pc), the input pointer (`0xC0458`, bus 0 = the PNO 2D block summed), the state pointer (the routine's `I1` context), the link `S - 0x1D` and the gain words (`0xB62D2` step, `0xB62D3` level; gain = clip(level + step) + 0.5). Messages write these words before pass 0, as the ARM does ([fx-chain-install](fx-chain-install.md) s3).
3. **Native side**: `fx_live_native` runs `FwReverb` (type 0, send 1 / dry 0) or `LayerFxChain` with `FirmwareFx::mod2` (CHOR, raw rate 64 amount 64) on the same WAV at the firmware frame rate 43,402.34375 Hz, so no rate conversion applies. Engine defaults and `render_hash` are untouched (D22).
4. **Metrics** (`tools/fx_oracle_live.py`, left channel): best integer lag within 16 frames, least-squares gain, RMS delta (dB), spectral centroid (Hz), tail length (time the 1 ms RMS envelope stays above -60 dB of its peak after the input ends), null depth (dB of the residual relative to the native RMS) with and without the fitted gain.

Entry registers of a stage routine (new `--regs-at PC`, VERIFIED run): `M0 = 0`, `M1 = 1`, `M2 = 2`, `M9 = 1`, `I1` = the context pointer, `I0 = 0xB62BE` (a scratch block in front of the dispatcher records, zero at entry), `I8` = the routine. These are the registers the `sharc-fx` golden harness sets, except `I0`.

## 3. Results

1.5 s burst (0.25 s of 440 Hz, amplitude 0.25, then silence), stereo, 3000 passes lead. Left channel, chain output of the oracle against the native model.

| Family | Lag (frames) | RMS oracle / native (dB) | Centroid oracle / native (Hz) | Tail oracle / native (ms) | Null depth, fitted gain | Verdict |
|---|---|---|---|---|---|---|
| pass-through (self check) | 0 | -37.4 / -22.8 (gain 0.1866) | 721.0 / 721.0 | 0.46 / 0.46 | **-157.6 dB** | harness VERIFIED |
| `reverb0` (Spring, type 0) | -16 | -5.5 / -24.9 | 9344 / 499 | 588 / 1250 | +19.4 dB (no match) | oracle response differs |
| `chor4` (Mod 2 CHOR) | 16 | -28.0 / -24.2 | 4994 / 2963 | 0 / 0.55 | -3.8 dB (no match) | oracle response differs |

What the oracle response is, measured with an impulse and with scaled input (`reverb0`):

- It is **linear in the input** (0.1 x input gives 0.1 x output to 6 digits) and **silent for silent input**, so it is not corrupted state noise.
- It starts **38 passes (152 frames) after the impulse**; the native `FwReverb` starts after 1352 calls (5408 frames), which the golden vectors of [fx-reverb-leslie](fx-reverb-leslie.md) give for the routine called directly. The live response then grows by about a factor 1e4 per 4096 frames and reaches 1e38 after about 1.1 s: an unstable feedback path, not a reverb tail.
- The `sharc-fx` golden vectors (`tests_fw_rvb_golden.rs`) were produced with the same routine, instance words and `M0 / M2 / M9`, with `I0` pointing at the 8 input words. In the live chain `I0` is the dispatcher scratch `0xB62BE` and the input pointer is read from the record; the bus `0xC0458` and the scratch both read zero when the probe pass reaches the routine.

Numbers of the native models alone on the same input (not hardware figures): `FwReverb` type 0 tail 1250 ms (the 1.5 s clip ends first), centroid 499 Hz; CHOR no tail, centroid 2963 Hz.

## 4. Which register or block stops the two families

The two routines are reached and run (`--regs-at 0x124AF2` and `0x123798` report their entry registers, the pass returns to idle every time). What blocks the comparison is the **calling convention of a stage routine inside the dispatcher**, not a missing peripheral: the census of the run (`--census`) lists only the boot handshake and the pass-start registers (`0x28B00B08` write per pass, `0x30FFE100`, `0x31002xxx`, `0x310C90xx` range blocks), no per-address check was made that none of them feeds the reverb (ASSUMED harmless: the instance and buffers are plain data RAM).

Candidates, in the order they should be tested (none verified):

1. **Scratch protocol.** The dispatcher hands `I0 = 0xB62BE` to the routine; the routine may expect the scaled input there while the dispatcher fills it from the bus only for stages the ARM installs through the full record (`fn, in, state, link, gain, accumulator`, [fx-dsp-map](fx-dsp-map.md) s4). The pass-through routine reads the input pointer itself, so it does not show the difference. Test: copy the 8 input words into `0xB62BE` at routine entry (a `--regs-at`-style hook) and compare with the golden call.
2. **Instance layout.** The golden data used the core 2 instance (`0x0802F000` lines); the live run uses the core 1 words of [fx-reverb-instance.csv](data/fx-reverb-instance.csv) (`0x08013000`). The buffers are zeroed by the script (VERIFIED the lines are then written by the run), but the core 1 pointers `0xB2849` (state array) and `0xE24E7` (all-pass buffer) may overlap live data of other stages.
3. **CHOR context.** `0xC13A4` (piano CHOR context) lies in the I/O block region of the input stage; the 1e38 values before the signal suggest a collision. Move the context and ring to free data RAM and retest.

## Implications

- The oracle can run the real FX chain on live audio and the chain transfer is exact (0.1866, lag 0, -157.6 dB), so the harness is a valid reference for any stage once its calling convention is reproduced. Reverb and chorus are **not yet reachable** through the stage install; D125 records the status and the next tests.
- No native FX model is changed and none is shown wrong: the native ports remain verified against direct routine calls ([fx-reverb-leslie](fx-reverb-leslie.md), [fx-mod2-validation](fx-mod2-validation.md)); this note adds no hardware evidence about them.
- Nothing here needs a hardware clip. A clip would only be needed after the oracle FX matches the ports and the question becomes hardware against firmware (OQ21c, OQ21a).

## Follow-ups

1. Test candidate 1 (scratch protocol) with a probe that fills `0xB62BE` at routine entry; if the response then starts after 1352 calls, add the scratch fill to `fx_live_run` as an install helper.
2. Move the CHOR context and ring to verified free RAM; run CHOR with the golden context words of [fx-mod2-validation](fx-mod2-validation.md).
3. Wire the PNO lane WAV (`sharc-sys --lane 2:0 --wav`) as the FX input once one family matches, and decide whether the 7-layer block needs the other layers.
4. ENS, FLANG, PHAS after CHOR.

## Reproduce

```bash
cargo build --release -p nord-sharc -p nord-tools
python tools/fx_oracle_live.py identity --image <fx>/core1_sram.bin --seconds 1.5
python tools/fx_oracle_live.py reverb0  --image <fx>/core1_sram.bin --seconds 1.5
python tools/fx_oracle_live.py chor4    --image <fx>/core1_sram.bin --seconds 1.5
target/release/sharc-fxlive <fx>/core1_sram.bin --core 1 --passes 3200 --regs-at 0x124AF2 --script-flat reverb0.script
```
