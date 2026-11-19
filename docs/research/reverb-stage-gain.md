# Reverb stage gain, send / return words, the second routine `0x1211C1` and the real rate

- **Checked:** 2026-11-17
- **Sources (local, never committed):** the FX Core 1 / 2 flat SRAM images (`python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram DIR`), `sharc-disasm` listings, the oracle `sharc-fxlive`, the ARM harness on the settled snapshot (`tools/fx_rvb_stage_capture.py`: units L4 and L10 piano, L6 synth; `target/bulk/boot_log.jsonl`, 6,264 FX messages of boot and settle). Committed (addresses, offsets and words only): [data/fx-stage-law.csv](data/fx-stage-law.csv), [data/fx-rvbm-ctx-access.csv](data/fx-rvbm-ctx-access.csv), [data/fx-rvb-stage-capture.csv](data/fx-rvb-stage-capture.csv). Builds on [reverb-fw-vs-hw](reverb-fw-vs-hw.md) (D145), [fx-dsp-map](fx-dsp-map.md) s4, [fx-oracle-live](fx-oracle-live.md), [fx-chain-install](fx-chain-install.md), [fx-contexts](fx-contexts.md), [dsp-frame-rate](dsp-frame-rate.md). No hardware audio was used.
- **Question:** where are the wet level and the send / return routing of the firmware multi-tap reverb set (gain word, writer, value per type and knob)? What is the second routine `0x1211C1`, what is the real reverb sample rate, and is the +21 / +26 / +36 dB excess of a Cathedral pad render a noise floor or a real gain?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Stage gain word | The dispatcher (`0x1217CB..0x12183B`) turns each stage record into a **cross-fade**: `c = clip(L + s, -lim, lim)`, `g = c + lim`, stage input `g * bus`, new bus `g * y + (lim - c) * bus`. Level `L = DM(S-0x1B)`, step `s = DM(S-0x1C)`, limit `lim = DM(S-0x19)` (image: `-0.5`, `-0.01`, `0.5`, so `g = 0`, the stage is bypassed). With `lim = 0.5` the stage output is at most `y(in)`: **no gain above unity**. The level ramps by `s` per pass | VERIFIED (oracle, 10 records, max error 3e-5, [data/fx-stage-law.csv](data/fx-stage-law.csv), `tools/tests/test_fx_stage_law.py`) |
| Writer of the stage words | **Not found.** 0 of 6,264 boot / settle FX messages and 0 of 2,088 reverb edit messages (units L4, L10, L6; types 2..11; amounts 0, 38, 127) write DM `0xB6000..0xB6FFF` (the stage records), a link, a level, a step or a limit word, or a pc of `0x120FDB`, `0x1211C1`, `0x123ABE`, `0x123C0F`, `0x124AF2`, `0x12417E`. The routine install of the reverb module is not run by the harness | VERIFIED (negative, `tools/tests/test_fx_rvb_stage_capture.py`, boot-log count by script) |
| What `kRvbAmt` writes | Exactly four words per edit, the same for every unit and type: send `c[-31]` and dry `c[-30]` of the multi-tap row and `c[18]`, `c[19]` of the Spring row (values: send `x^2 (2 - x)`, dry equal power; 0, 38, 127 give send `0`, `0.1531`, `1`). No type, amount or enable edit writes `c[-28]`, `c[42]`, `c[43]` or a stage record word | VERIFIED (run, 30 edit sets) |
| Who reads them | The core `0x120D8A` reads `c[-31]` (target) and smooths it into `c[-29]` with `c[-32]` (0.9913 per call) and scales the wet by `c[-29]`. **No instruction of the module reads `c[-30]`** (the dry word). The wrapper `0x120FDB` and `0x1211C1` read only `c[-28]` (dry mix, **0 in the image, never written**), `c[42]`, `c[43]` (output scale per channel, 1.0) | VERIFIED (listing scan, [data/fx-rvbm-ctx-access.csv](data/fx-rvbm-ctx-access.csv), `tools/tests/test_fx_rvbm_ctx_access.py`) |
| Consequence for the routing | The routine output is **wet only** (`in * 0 + wet`), scaled by the send. The dry never passes through the routine: it is the `(lim - c) * bus` term of the dispatcher, i.e. set by the stage level word, not by the ARM `dry` word that `kRvbAmt` writes. At hardware knob 3 the wet is `send` (0.153) times the routine response, as the port has it | VERIFIED (listing) + EMPIRICAL (reading) |
| `0x1211C1` | A 2-frame variant of the same effect: reads words 0..1 and 2..3 of the block directly (no fs/4 pre-bank), calls the same core `0x120D8A` once, mixes `in * c[-28] + wet`, scales L by `c[42]`, R by `c[43]`, writes words 0..3 only (no post-bank). 17 instructions | VERIFIED (listing); role and trigger open |
| Real reverb rate | The routine runs once per 4-frame block (10,850.6 Hz call rate at the verified frame rate 43,402.34 Hz). The wrapper low-passes the block with three fs/4 biquads, hands the core frames 0 and 2 (**21,701.17 Hz**, half the frame rate) and holds each result for two frames. The real tail of the port is therefore 1.19 s x 48,000 / 43,402.34 = **1.32 s** in time, not 1.19 s | VERIFIED (frame rate, [dsp-frame-rate](dsp-frame-rate.md)); EMPIRICAL (decimation reading from the listing) |
| The +21 / +26 / +36 dB | **Not a gain, a signal-relative floor.** Stage gain is at most unity (above), the routine's in-band power gain is `-3.8 .. -10 dB` below 2 kHz and under `-20 dB` above 4 kHz ([reverb-fw-vs-hw](reverb-fw-vs-hw.md) s2), and the out-of-band floor of a tone stays a fixed fraction of the tone (-55.75 dB at amplitude 0.3 and 0.03, -54.19 dB at 0.003, `tests_fw_rvbm_floor.rs`). The pad render itself was not reproduced | VERIFIED (tests) for the floor and gain bounds; ASSUMED that the pad's dry bands are empty enough |
| Native change | None. No default flips (D22) | - |

## 2. The dispatcher stage law (VERIFIED)

Listing of the pass routine (core 2 byte addresses `0x242F96..0x24306A`; core 1 is the same code `0x974` instructions later; one group of 35 stage calls):

```
R10 = DM(-27, I1)   R11 = DM(-28, I1)   R7 = DM(-25, I1)      ; level L, step s, limit lim
F10 = F10 + F11     F11 = CLIP F10 BY F7                      ; c = clip(L + s, lim)
DM(-27, I1) = R11                                             ; level <- c  (ramp by s per pass)
I0 = DM(-31, I1)    I8 = DM(-32, I1)    R12 = DM(-30, I1)     ; input block, routine, state pointer
F10 = F11 + F7                                                ; g = c + lim
scratch (0xB62BE) = g * bus ; F11 = F7 - F11 ; DM(0xB62BA) = F11 ; CALL (M8, I8)   ; k = lim - c
... bus(DM(I0)) = y * g + DM(-24, I3) * bus     ; I3 = 0xB62D2, so DM(-24, I3) = DM(0xB62BA) = k
```

`I3` was read at the first call of a run (`sharc-fxlive --regs-at`): `I3 = 0xB62D2`, so the dry term is the saved `k = lim - c`, not a layer bus gain. A pass-through routine (`y = g * bus`) therefore has the bus gain `g^2 + lim - c`: 1.0 at `g = 0` and `g = 1`, 0.75 at `g = 0.5`.

Oracle check (`python tools/fx_stage_law.py probe`, stage 0 of core 1, pass-through routine, 440 Hz, gain relative to the unity record):

| level | step | limit | g (first pass) | expected | measured |
|---|---|---|---|---|---|
| -0.5 | 0 | 0.5 | 0 | 1.0 | 1.0 |
| -0.25 | 0 | 0.5 | 0.25 | 0.8125 | 0.812478 |
| 0 | 0 | 0.5 | 0.5 | 0.75 | 0.75 |
| 0.25 | 0 | 0.5 | 0.75 | 0.8125 | 0.812478 |
| 0.5 | 0 | 0.5 | 1 | 1.0 | 1.0 |
| -0.25 / 0 / 0.25 | 0 | 0.25 | 0 / 0.25 / 0.5 | 0.5 / 0.3125 / 0.25 | 0.5 / 0.312478 / 0.25 |
| -0.5, 0.25 | 0.25 | 0.5 | ramps to 1 | 1.0 | 1.0 |

Meaning for the reverb: a stage that is on at `g = 1` **replaces** the bus by the routine output. The multi-tap routine returns wet only, so a hardware reverb with `g = 1` would have no dry at all; any audible dry (the manual's mix knob) needs `g < 1`, i.e. a level word the harness never receives (section 3). The port and the engine mix dry and wet themselves, which is the same balance as the hardware only if the ARM drives `g` from the knob.

## 3. What the ARM writes (VERIFIED run, negative results)

`python tools/fx_rvb_stage_capture.py CODE SRAM --cache target/settled.snap --units 4,10,6 --types 2..11 --amts 0,38,127` applies `kRvbEnable`, `kRvbType`, `kRvbAmt` to the three effect units through the registry dispatcher and records every DSP word with its writer pc and the owning state row (`tools/fx_contexts.py::locate`). Result ([data/fx-rvb-stage-capture.csv](data/fx-rvb-stage-capture.csv), 2,088 rows without the enable step):

| Step | Words | Reading |
|---|---|---|
| `kRvbAmt` (3 amounts x 10 types x 3 units) | 4 words: row `0xC1C0C` (L4) / `0xC1B6C` (L10) / `0xC1A28` (L6) `c[-31]`, `c[-30]` and the Spring row `c[18]`, `c[19]` | send / dry words, VERIFIED values |
| `kRvbType` | type tables (PM `0x94405..`, `0x9440E`, `0x9441F`), the row's feedback word `c[-27]` (0.35 type 8, 0.4549 image), the colour words | per type, unchanged by the amount |
| any | 0 words in `0xB6000..0xB6FFF`, 0 to `c[-28]`, `c[-25]`, `c[42]`, `c[43]` of a multi-tap row | no stage level, no dry mix, no output scale per type or knob |

The two words the dry / return side would need are **not edit-driven**: `c[42]` and `c[43]` are written 1.0 at boot by the ARM function `0xC0212400..0xC02126C0` (`pc 0xC0212644`, `0xC0212488`; message ids row `+0x128` / `+0x12C`, value `0x3F800000` or 0 by a per-unit flag byte `[table + 0x90 + L]`, `L <= 11`): an on / off style return switch, not a level law. `c[-28]` stays the image value 0.

What this leaves open (OQ21c): the writer of the stage level / limit words (not an edit id; the record install and the part activation are not run by the harness), hence the real dry / wet balance and wet level at knob 3, and the install of `0x120FDB` / `0x1211C1` into stage records. The only wet-level facts that hold are: the routine wet is `send x response`, the stage gain is bounded by `(2 lim)^2` of the routine (unity at the image limit), and no ARM edit raises either.

## 4. `0x1211C1` (VERIFIED listing, role open)

```
R12 = DM(0, I0)   R14 = DM(2, I0)        ; words 0..1 and 2..3 of the block: frame 0 and frame 1
CALL 0x120D8A                              ; the same packed core, one call
R2 = DM(-28, I1)                           ; dry mix (0 in the image)
F0..F1 = block words * c[-28] + (R12, R14) ; wet added to the dry copy
F0..F1 *= c[42] (L) / c[43] (R)            ; output scale (R5 <-> S4 swap in SIMD mode)
DM(0, I0) = R0   DM(2, I0) = R1   RTS
```

Against the main routine `0x120FDB` (4 frames: three fs/4 biquads in, core on frames 0 and 2, hold, three biquads out): `0x1211C1` has no filter banks, takes consecutive frames (0 and 1) instead of every second one and returns 2 frames. It reads the same three context words ([data/fx-rvbm-ctx-access.csv](data/fx-rvbm-ctx-access.csv): wrapper and second routine both load exactly `{-28, 42, 43}`). The directory lists the pair as sizes `2` (`0x120FDB`) and `1` (`0x1211C1`) in one module, like the Amp pair `0x124D68` / `0x124E0B` whose two parts the ARM installs into different slots; no word with either pc appears in any message (section 1), so which one runs in a given reverb mode is not decided here. EMPIRICAL guess: a low-rate, filter-free instance fed by a mono-summed or already band-limited bus. Not ported (a Rust port needs the install path first).

## 5. The real sample rate (VERIFIED frame rate, EMPIRICAL reading)

- Frame rate: 43,402.34 Hz ([dsp-frame-rate](dsp-frame-rate.md)); one pass, one block of 4 frames, one call of the routine.
- The three input biquads are Butterworth sections at fs/4 (`FW_BUTTERWORTH_FS4_WORDS`, [fx-dsp-map](fx-dsp-map.md) s6): a half-band low-pass. The core is then handed `DM(0, I2)` and `DM(4, I2)` of the filtered block (frames 0 and 2), a decimation by 2: **core rate 21,701.17 Hz**, Nyquist 10,850.6 Hz = the fs/4 corner. Each core result is stored twice (`DM(0)`, `DM(2)` and `DM(4)`, `DM(6)`) and the output bank smooths the hold.
- The port runs this block code on the engine's 48 kHz, so every time constant of the firmware reverb is 10.6 % too short in real time: tails `1.19 s -> 1.32 s` (Hall and Cathedral share it), first echoes 180 / 252 ms -> 199 / 279 ms. `fw_rate_match` is not applied to the reverb. The 43,402 Hz question of OQ21c is answered; the by-ear gap (tail 3.4 / 7.9 s in the engine) is **not** explained by it (a factor 1.11 against 2.6 and 6).

## 6. Noise floor or gain (VERIFIED bounds)

| Claim | Test | Result |
|---|---|---|
| The stage cannot add gain | `tools/tests/test_fx_stage_law.py` (law and oracle table) | `g <= 2 lim = 1` at the image limit; bus gain `<= 1` for pass-through |
| The routine has no gain | `tests_fw_rvbm_levels.rs` (D145) | wet-only power gain `< 0.35` and level independent within 0.1 dB |
| A floor outside the tone's band exists | `tests_fw_rvbm_levels.rs`, `tests_fw_rvbm_floor.rs` | below `-20 dB` re the tone (second difference above about 3 kHz) |
| The floor is a fixed fraction of the signal | `tests_fw_rvbm_floor.rs` | the out-of-band / total power ratio of a 220 Hz tone is -55.75 dB at amplitude 0.3 and 0.03 (equal within 0.1 dB) and -54.19 dB at 0.003 (a small absolute part; an absolute floor would move 20 dB per decade) |
| An empty dry band turns it into a large ratio | `tests_fw_rvbm_floor.rs` | a dry band 40 dB under the tone hides it, 100 dB under shows `+20 dB` or more |

So the reading of D145 is upgraded from "consistent with a noise floor" to: **the excess is the wet to dry ratio of a signal-relative floor against a nearly empty dry band, not a stage gain and not a larger routine gain**. Still unproven: that the pad render's dry top octaves are that empty (no render was reproduced); it would need the original render.

## Implications

- `--firmware-reverb` stays opt-in (D22, D31, D136, D145): no stage gain can explain the by-ear gap; the open item is the stage level (dry / wet cross-fade) the ARM sets when it installs the module, not a gain above unity.
- The 15 dB wet deficit at knob 3 is the send law (0.153) times the routine's own IR energy (-19.4 dB re input against the engine FDN's -10.4 dB). A level-only fix would hide a structural difference (D145 alternatives).
- Reverb time scale: the firmware tail is 1.32 s in real time, not 1.19 s.

## Follow-ups

1. Find the stage-record install: run the part activation / module-directory constructors in the ARM harness (the descriptors `[[obj+4]+4]` are zero, [fx-chain-install](fx-chain-install.md) s4) and record the level, step, limit, link and routine words per reverb type; then run the multi-tap stage in `tools/fx_oracle_live.py` with those words (a `rvbm` family on core 1).
2. A hardware clip of a held key into Cathedral at knob 38 and 127: wet level, tail (1.32 s predicted) and dry level.
3. Apply `fw_rate_match` to `FwReverbMulti` behind an opt-in flag once the install path fixes the block timing (the routine scales with the block rate).
4. Port `0x1211C1` once a reverb mode that installs it is found.

## Reproduce

```bash
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_0_FX.bin --sram DIR
python tools/fx_stage_law.py probe --image DIR/core1_sram.bin --sharc target/release/sharc-fxlive --out docs/research/data/fx-stage-law.csv
sharc-disasm DIR/core2_sram.bin 0x241B14 0x2423C4 > rvbm.lst
python tools/fx_rvbm_ctx_access.py rvbm.lst --out docs/research/data/fx-rvbm-ctx-access.csv
uv run --with unicorn --with capstone python tools/fx_rvb_stage_capture.py CODE SRAM --cache target/settled.snap --units 4,10,6 --types 2,3,4,5,6,7,8,9,10,11 --amts 0,38,127 --out cap.csv
cargo test --release -p nord-core --lib tests_fw_rvbm_floor
```
