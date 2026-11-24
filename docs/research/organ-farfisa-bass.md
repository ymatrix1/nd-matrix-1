# Research: Farfisa (`0x122A71` + `0x122B85`) and B3 Bass (`0x122296`) in the oracle

- **Checked:** 2026-11-13
- **Sources (local, never committed):** `target/fw_unpacked/bank0_CODE.bin` / `bank0_SRAM.bin` (OS v1.66), the PNO image `SRAM/core1_sram.bin` (`tools/ns4_dsp_images.py`, `tools/ns4_dsp_ldr.py --sram`) and its ADI `elfdump` listing (`tools/adi_dis.py`, byte = 2 pc), the oracles `sharc-pass` and `sharc-chain` (`crates/nord-sharc`), the booted-ARM harness (`tools/arm_edit_capture.py`, `arm_model_paths.py`, `organ_b3_pass.py`), `capstone` listing of the ARM code, the user manual (`.reference/`, p. 7, 18, 21). Builds on [b3-tone-source](b3-tone-source.md), [organ-model-paths](organ-model-paths.md), [pno-full-pass](pno-full-pass.md), [b3-generator](b3-generator.md), [organ-post-stage](organ-post-stage.md). Derived data: none new (all numbers are in the tables below).
- **Question:** what do the Farfisa generator `0x122A71` and processor `0x122B85` and the B3 Bass processor `0x122296` compute, can their pass be replayed with the ARM's own messages, and which part is exact enough to port (OQ18)? Also: who owns the two event-2 chain objects, and does the value order of `Edt_Org_kModel` follow the UI?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command below, **EMPIRICAL** = fitted or read from data or a listing, **ASSUMED** = a reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Farfisa tone source | **The generator `0x122A71` is a free-running top-octave divider**: 12 phase accumulators (one per semitone), each with a ladder of 8 rows (9 for the two lowest), every row a soft pulse wave read from the doubled phase. It has no key input; the keys act in the processor `0x122B85` through ARM weights over the generator ring. Python model `tools/organ_farfisa_ref.py`: **bit-exact** against the oracle on 4 random scenarios and on 40 consecutive calls of the image state (7,680 + words, 0 differences). | VERIFIED (`organ_farfisa_oracle.py verify`) |
| Farfisa processor | 4 weighted sums of the ring (4 x 94 weights, `DM 0x95FFC..`), two short biquad groups, 8 lattice sections selected by a flag, 4 gains (`DM 0xC27E0..E6`), a DC blocker; two frames per pass. **Everything it needs is zero in the image and in every captured ARM burst** (weights `0x95FFC..0x96173`, lattice table `0x965FC..`): with the model install, the engine constructor and a chord the full pass is silent; with ASSUMED uniform weights it stays silent (the lattice table is zero too). The ARM writer of these words is open (OQ19). | VERIFIED (negative), structure EMPIRICAL |
| B3 Bass | Generator `0x121BBE` (the B3 wheels of [b3-generator](b3-generator.md)) + processor `0x122296`. **The complete pass sounds**: chord C4 E4 G4 with the ARM key-down gives lines at 262.0, 330.2 and 392.2 Hz (0, -3.9, -6.4 dB), the 8' partial of each key at the assumed frame rate, no line within 30 dB at 16' (the bass processor sums two short ring windows, notes `n` and `n + 12`, 13 and 16 loop iterations, not the whole ring; ASSUMED reading). | VERIFIED (run) |
| B3 (model 0) with the same ARM bursts | Silent: the dry gain `DM 0xB44C0` is 0 (`f0 += s DM(0xB44C0)`); poking `1.0` gives 131.8, 259.6 and 391.4 Hz for 16', 8' and 5 1/3' of key 60. The Bass has no such gain word, hence it sounds. | VERIFIED (run) |
| Why the previous replays were silent | The booted-ARM harness never ran the engine constructor `0xC002187C`, the only caller of the bulk sender of `DM 0x95471..` (33 / 61 / 14 tap tables and the per-model words, 15,289 words). With the constructor messages first, the B3 / Bass chains have their FIR and lattice data. | VERIFIED (run) |
| Event-2 chain objects | **The second store of each organ layer.** Every organ layer client has two stores; `[store + 0x98]` of the first is the chain object that `Edt_Org_kModel` writes (event = model; chan 0 and chan 2), `[store + 0x98]` of the second is a chain object whose event is **2 permanently** (chan 1 = `0x90fff660`, store `0x90ffd720`; chan 3 = `0x91021e90`, store `0x9101ff50`). They install the Farfisa generator in slot 0 and the Farfisa processors in slots 5 and 7 once, at boot. What the second store edits is not identified. | VERIFIED (run, section 6) |
| `Edt_Org_kModel` value order against the UI | The firmware string pool order is `B3, Vox, Farfisa, Pipe1, Pipe2, B3 Bass`. The manual lists the models in the same order (p. 7, 18: "tonewheel/B3 with a separate B3 Bass mode, two transistor organs (Vox and Farfisa), two Pipe organs"; p. 21: B3 Bass is the 16' and 8' bass drawbar model). Supports the order; the UI selector index itself is not read. | EMPIRICAL |
| Rust port | Opt-in `ToneWheelSource::FarfisaWheels` (`organ/farfisa_wheels.rs`): the generator, closed-form tables, 91-wheel bus. Against the oracle ring over 60,000 frames: max |diff| 4.0e-5, SNR >= 104.5 dB, spectral error 0.00 dB. The processor is not ported (no weights). | VERIFIED (section 5) |

## 2. The ARM side and the replay

```bash
uv run --with unicorn --with capstone --with numpy python tools/organ_fb_pass.py script \
    target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin fb_m5.script \
    --model 5 --keys 60,64,67 --cache target/settled.snap       # 23,255 words (model 2: 23,444)
python tools/organ_fb_pass.py run SRAM/core1_sram.bin fb_m5.script --exe target/release/sharc-pass
```

Order of the messages: the engine constructor `0xC002187C` (the only caller `0xC0248CFC`; it sends 32 words of taps `DM 0x95471..` through the bulk sender `0xC001C6A0`, the image defaults of the processors and 15,289 PNO core 1 words in all), the first windows of the booted main loop (default model B3 on both layers), `Edt_Org_kModel = N` on layer 0, the drawbar update of record 0, then the key-down of the drawbar and percussion items of each key (`0xC00BF890`). Model 2 install: 526 words, model 5: 337.

| Model | Slots after the install (layer A) | Output of the pass (set 2 pair 0, 8,000 passes) |
|---|---|---|
| 0 B3 | `0x122A71`, `0x121BBE`, idle, idle, `0x121E2B` (`I1 = 0xB472C`), `0x122B85`, `0x121E2B`, `0x122B85` | silent (dry gain 0) |
| 2 Farfisa | slot 4 = `0x122B85`, `I1 = 0xB474C` | silent (weights and lattice table zero) |
| 5 B3 Bass | slot 4 = `0x122296`, `I1 = 0xB472C` | lines 262.0, 330.2, 392.2 Hz, peak 0.0083, rms 0.0031; side lines about 7 Hz away (-15 dB) |

The frame rate behind the Hz values is the anchor of section 5 (43,402.6 Hz, ASSUMED). The side lines of the Bass (254.2 and 268.2 Hz around 262.0, same offsets around 330.2) are the tapped delay of section 4.2.

## 3. The Farfisa generator `0x122A71` (listing, model, tables)

Registers: SIMD pair `p` holds oscillator `2p` in PEx and `2p + 1` in PEy. Words: LFO rate `DM 0xC27D6`, depth `0xC27D7`, phases `0xC27D8 + lane`, fine word `0xC27D4` (image `0x40000000` = 0.5), polynomial `A, B, C` pairs at `0xC27DA..DF`, pointers (long words) T `0xC27D0` = `0xC2950`, counts `0xC27D1` = `0xC2C1A`, stream S `0xC27D2` = `0xC2AD8`, ring `0xC27D3` = `0xC2B52`. Verified (the model reproduces every word the routine writes):

```text
lfo (lane l)  ph_l += rate (lane 1 also uses lane 0's rate: `r11 <-> s1` runs in PEx only)
              tri = 2 |ph_l / 2^31| - 1;  lfo = A tri + B tri^3 + C tri^5;  m_l = 1 + lfo depth / 2^31
step          K_l = fix(fine m_l 2^31)        (lane 1 multiplies lane 0's fine word)
pair j, lane l, frame f in {0, 1}, with n = 2j + l:
              phase += ssf(K_l, inc_n)
              row r < N_j:   u = (|phase_r / 2^31| + off) slope_(n,r)
                             y = max(b_(n,r) clip(u) + a_(n,r) y_prev, clip(u))      -> ring_f[12 r + n]
                             phase_(r+1) = 2 phase_r + S_(n,r)                       (32 bit wrap)
              N_j = 9 for j = 0, else 8 (DM 0xC2C1A..1F; the loop counter is PEx r3)
```

`y_prev` is the same ring word of the *other* frame (`r2 = dm(m4, i5)` with `m4 = +0x62` in frame 0 and `m5 = -0x62` in frame 1): a one-pole release per slot. Frame 0 writes the ring at `0xC2B52`, frame 1 the ring at `0xC2BB4`, so one call advances the phases twice (2 frames per call, as everywhere in the pass). Memory use per pair: stream `4 + 2 N` words (`inc` x 2, phase x 2, then one offset pair per row, 22 words for pair 0, 20 for the others), table `8 N` words (`off, slope, a, b` as pairs).

Tables (fits against the 108 rows and 12 stream words of the image, `python tools/organ_farfisa_oracle.py tables SRAM/core1_sram.bin`):

| Item | Closed form | Error against the image | Tag |
|---|---|---|---|
| `inc_n` | `round(0x316160 2^(n/12))` (12-TET) | 1 unit on 4 of 12 words (3e-7, 0.0005 cent) | VERIFIED |
| offset of every row | `S = 0xAAAAAC11` (`-0.6667 2^31`) | exact | VERIFIED |
| `off` | `0xBF2AAAA7` (-0.66666645) | exact | VERIFIED |
| `slope_(n,r)` | `-1327.1669 2^-r 2^(-n/12)` | 1.0e-7 relative | VERIFIED (fit) |
| `a_(n,r)`, `b = 1 - a` | `exp(-0.0047343 2^r 2^(n/12))` for `r <= 6`; `r = 7`: exponent `2^11 2^(n/12)` (`a` of 6e-5 .. 1e-8); `r = 8`: `a = 0`, `b = 1` | 1.0e-6 on `a` | EMPIRICAL (fit) |

The pulse: `u = (2/3 - |x|) |slope|` is positive for `|x| < 2/3`, so a row is high for 2/3 of the cycle and low for 1/3, with edges `1 / |slope|` of a cycle wide (2 frames for the lowest row, constant in samples for every row because the slope halves as the frequency doubles). Row 0 of oscillator 0 is C0 = 16.35 Hz at the anchor rate; row `r` of oscillator `n` is MIDI note `12 + n + 12 r` (rows 0..7, rows 8 only for `n < 2`, up to note 109).

Ladder, edge and release together make the divider band limited by the slew of the edge (2 frames) and give the high rows a soft decay of the low level (the `a` weight of the previous frame, time constant `1 / (0.0047343 2^r 2^(n/12))` frames: 211 frames for C0, 3.3 frames at row 6).

Frame rate: oscillator 0 steps `0x18B0B0` units per frame (3.76737e-4 cycles); at C0 = 16.3516 Hz that is **43,402.6 Hz**. The same factor to the 43,156.4 Hz of the bank derivation is 0.99430 (-9.9 cent), the Vox offset of [pno-full-pass](pno-full-pass.md) s4: both models are consistent with a DSP frame rate of about 43,403 Hz, and the Pipe bank, tuned exactly at the 43,156.4 Hz of `lane_mix.rs`, would then be 9.9 cent sharp. ASSUMED (needs a recording), but two independent models agree.

## 4. The processors (listing structure, EMPIRICAL)

### 4.1 Farfisa `0x122B85` (359 instructions, `elfdump` listing)

| Part (listing pc) | What it does |
|---|---|
| `0x122B85..0x122B9B` | record `I1 = 0xB474C + k`: `{i2 = param block, i3 = state block, i10 = PM weights, i11 = PM filter table}` at `I1 - 0x20..-0x1D` (layer A: `0xC2D78`, `0xC2C40`, `0x95FFC`, `0x965FC`; the four records are the four processor slots 4..7: `0xB472C + 4 i`); ring pointer `DM 0xC2C16`, weight pointer `DM 0xC2C18`; frame loop x 2 |
| `0x122BAB..0x122C17` | four weighted sums over the ring pairs (hardware loops of 20, 23, 23 and 22 iterations of two pair steps with `pm(i10)` weights; the last window starts at `0xC2B56`); the weight array of a record is `0x178` words (record spacing `0x95FFC`, `0x96174`, `0x962EC`, `0x96464`), i.e. 4 x 94 |
| `0x122C18..0x122D12` | the four sums are weighted by the param block (`dm(i2 - 0x20..)`), lane sums, two short sections with states in `i3`, then `if ne jump 0x122D88` picks the lattice table `pm(i11 - 0x1E)` or `pm(i11 - 0x1C)` |
| `0x122D18..0x122E55` | lattice sections (`f8 = f0 f3; f13 = f0 f4; ... f1 = f3 - f4`, unrolled 4 or 8 per branch, coefficients `pm(i6)`, states `dm(i7)`) with the output gains `DM 0xC27E0 / E2 / E4 / E6` between the groups |
| `0x122E56..0x122EF5` | coupled sections with states in `dm(i4)`, `0.99800` (`0x3F7F6944`) as a DC-blocker-like constant, the final `dm(i0, m2) = r0` into the slot stream; ring pointer `+ 0x62` for frame 1 |

This table is a reading of the listing (EMPIRICAL), not a model. The weights, filter tables and the lattice selection flag are ARM programs (zero in every captured burst). A bit-exact model on random inputs is possible (the structure is regular) and is the next step; without real coefficients it cannot be compared with audio.

### 4.2 B3 Bass `0x122296` (367 instructions)

| Part | What it does |
|---|---|
| `0x122296..0x1222B0` | same record layout as the B3 processor; saves `I0` in `DM 0xB33C0`, the ring index `DM 0xB33BA` and the frame counter `0xB33BC` are zeroed and advance `+0x68` / `+2` per frame |
| `0x1222B3..0x122333` | two dot products of the B3 generator ring `0xB32E8 + index` (note `n`) and `0xB32F4 + index` (note `n + 12`) with `pm(i10)` weights (13 + 3 and 16 + 3 pair steps): only the 8' and 16' band of the ring contributes (the manual: B3 Bass = 16' and 8' bass drawbars, p. 21) |
| `0x122335..0x122343` | the two sums plus two words from the param block (`dm(m3, i6)`) |
| `0x122343..0x122376` | 33-tap FIR (`DM 0x95471..`, circular `l6 = 0x21`) |
| `0x122380..0x122498` | two lattice groups (`DM 0x95491`, `0x955B0`) writing a delay line at `DM 0xB46E6..` |
| `0x1224D2..0x122542` | 9 taps of that delay line at the indices `m3 = dm(0x10..0x18, i2)` of the param block, weighted by 9 coefficients at `DM 0xC0202..`: a tapped delay, i.e. the Hammond scanner vibrato / chorus (V1..V3, C1..C3), ASSUMED role; the replay shows side lines about 7 Hz from each chord tone |
| `0x122545..0x12261D` | three biquad sections, a DC blocker (`0x3F7D70A4`), the gain `dm(0x1E, i3)`, the store `dm(i0, m2) = r0` and the ring bookkeeping |

This also explains the "line box tail" of [b3-generator](b3-generator.md) s4: `DM 0x95471..0x954E5` are the FIR and lattice data of the **scanner** in the processor, written by the engine constructor.

## 5. Port and oracle comparison

```bash
python tools/organ_farfisa_oracle.py verify SRAM/core1_sram.bin --exe target/release/sharc-chain
python tools/organ_farfisa_oracle.py tables SRAM/core1_sram.bin
cargo run --release -p nord-tools --bin farfisa_ref -- 60000 farf_rust60k.csv
python tools/organ_farfisa_oracle.py compare SRAM/core1_sram.bin --exe target/release/sharc-chain \
    --frames 4000 --rust farf_rust.csv                      # Python model + Rust
```

| Check | Result |
|---|---|
| Python model against the oracle | 4 random scenarios (random tables, counts 2..8, phases, LFO): 0 differing words; image state, 40 consecutive calls: 0 differing words |
| Closed-form Python model against the oracle ring, 4,000 frames, 98 slots | max abs 1.2e-5, worst SNR 105.4 dB, spectral error 0.00 dB (bins within 60 dB of the peak) |
| Rust `FarfisaWheels` against the oracle ring, 60,000 frames, 98 slots | max abs **4.0e-5**, worst SNR **104.5 dB**, spectral error **0.00 dB** |
| Rust against the Python golden vectors (`farfisa_golden.rs`, 3 frames, 3 oscillators) | below 2e-4 |
| Default render | `render_hash` unchanged: `25baa387b0219c0835094150106a35ce751b9c16b68109d2a030596e78e7b59a` |

Host rate: the Rust source keeps the pitch (12-TET, A = 440) and the edge and release widths in samples at any sample rate (`rho = 43,402.6 / fs`). Wheel `w` of the bus is note `24 + w`: wheels 0..85 are filled, 86..90 (notes 110..114) are silent (the divider has no note there), the lowest row (notes 12..23) is dropped.

## 6. Owners of the event-2 chain objects (`python` one-off, `tools/arm_edit_capture.py`)

`aec.find_stores` of the settled state lists two stores per organ layer (`0x90ffab40` / `0x90ffd720` for layer 0, `0x9101d3d0` / `0x9101ff50` for layer 1). `[store + 0x98]`: `0x90ffca80` (event 0), `0x90fff660` (event 2), `0x9101f310` (event 0), `0x91021e90` (event 2). `Edt_Org_kModel` (`0xC011C9BC`) only writes the first store's chain. The boot slot table of [b3-tone-source](b3-tone-source.md) s4.1 follows: chan 1 and chan 3 write generator slot 0 and processor slots 5 and 7. The piano layers share one chain object (`0x90c81490`, event 22). What the second store represents (a second organ engine: percussion or the bass drawbars) is open.

## 7. Corrected-state oracle chords and the processor inputs (2026-11-13 follow-up)

Method: `tools/organ_proc_oracle.py` (script = full boot log without `DM 0xB2ECF` / `0xB2FF0` + model install + key-down words, [arm-boot-state](arm-boot-state.md) s4), `sharc-pass` PNO core 1, 3,000 passes, set 2 pair 0 lane L, lines at the 43,402.6 Hz anchor (bin width about 8 Hz, so the Hz values are quantised; use the table for levels). New oracle option `sharc-pass --rec ADDR:COUNT:FILE` writes DM words after every pass (ring, state, slot stream, for later model checks). Table: [organ-proc-oracle-lines.csv](data/organ-proc-oracle-lines.csv).

| Model, chord (keys) | Peak | Strongest lines (Hz at dB below the strongest) | Tag |
|---|---|---|---|
| Farfisa, 60 64 67 | 0.0867 | 258.7 (0), 392.3 (0), 133.5 (0), 166.9 (-1) | VERIFIED (run) |
| Farfisa, 48 55 64 | 0.0806 | 133.5 (0), 66.8 (-3), 100.2 (-3), 192.0 (-3) | VERIFIED (run) |
| Farfisa, 72 76 79 | 0.0693 | 784.6 (0), 659.4 (-1), 525.8 (-5), 1043.3 (-7) | VERIFIED (run) |
| Farfisa, single key 60 | 0.0405 | 131.8, 263.5, 523.2, 786.7 Hz (0, -1, -7, -9 dB) | VERIFIED (run, reproduces the cited figures) |
| B3 Bass, 60 64 67 | 0.0679 | 258.7 (0), 325.5 (-2), 392.3 (-2), 525.8 (-38) | VERIFIED (run) |
| B3 Bass, 48 55 64 | 0.0892 | 192.0 (0), 133.5 (-1), 325.5 (-3), 392.3 (-37) | VERIFIED (run) |
| B3 Bass, 72 76 79 | 0.0487 | 525.8 (0), 659.4 (-4), 784.6 (-5), 1043.3 (-40) | VERIFIED (run) |

What the Farfisa processor reads (VERIFIED by the runs above and by `sharc-chain` pokes):

| Input | Value / source |
|---|---|
| Record `I1 = 0xB474C` (layer A, slot 4) | `{i2, i3, i10, i11}` at `I1 - 0x20..-0x1D` = `0xC2D78, 0xC2C40, 0x95FFC, 0x965FC` (image defaults; boot, install and key-down never write the record) |
| Weights `DM 0x95FFC + 94 s + idx` (sum `s` = 0..3) | zero in image, boot and install; **key-down only**: key `n` writes 1/48 (`0x3CAAAA80`) at `0x95FE4 + n`, `+ 0x62`, `+ 0xD0` (sum 0 idx `n - 24`, sum 1 idx `n - 20`, sum 2 idx `n - 4`) and one bias word each at `0xC2D58, 0xC2D59, 0xC2D5C` (-0.00958, the same for every key; `keydown-organ-models.csv` notes 36, 60, 84). A probe with one weight and one ring slot set to 1.0 shows sum 0 idx 10 reads ring slot 10 and sum 1 idx 10 reads ring slot 18; sums 2 and 3 were not resolved (the output is delayed by the filters) |
| Lattice table (`i11 = 0x965FC`) | the processor reads only `pm(i11 - 0x20 .. -0x1C)`: a pointer table `0x965DC..` that is **in the image** (DM pointers such as `0xC2CD8, 0xC2920, 0xC27E8, 0xC2860`), not zero. The 204 boot words at `0x965FC..` belong to another consumer. The coefficient banks the pointers address (`0xC27E0..0xC2D78`) are written by the **boot** (272 words, 142 non-zero) and the **model install** (180 words, 129 non-zero); the state block `0xC2C40..` by the boot (46 words) |
| Output | `dm(i0, m2) = r0` once per frame (SIMD: PEx lane at `i0`, PEy lane at `i0 + 1`), two frames per call; from zero state the first calls are about 5e-6 |

Who writes "the rest of the lattice table" (OQ19): nothing is missing. The old silent Farfisa pass needed only the key-down weights on top of the corrected boot log; the claim of s1 that the table `0x965FC..` is zero concerned a range the processor does not read.

Not done in this follow-up (stop point at the 90 minute time box): bit-exact Python models of `0x122B85` / `0x122296` (SIMD listing with PEx / PEy swaps `r5 <-> s4`, circular buffers and a record-flag selected lattice; `sharc-chain --trace` gives every register per instruction, so a model can be checked step by step), the Rust processor stages, the line-by-line Rust comparison, `emu_vs_hw.py` for Farfisa and the video check. Defaults and `render_hash` are unchanged. The processors are linear in the ring and the key-down weights (the lattice flag is a record word, not a signal), so the measured-response option of `b3_tail.rs` applies to both; the exact model stays the target.

## 8. Measured line gain of the Farfisa processor (EMPIRICAL)

Tool: `tools/organ_farfisa_proc.py measure` (`sharc-chain`: generator `0x122A71` + processor `0x122B85` per call from the boot-plus-install state, all weights zero but one word 1.0, 4,000 calls, ring slot and output recorded together, Hann FFT, gain `|Y| / |R|` at every ring line within 50 dB of the peak, 22 slots per sum, binned to 1/6 octave). Data: [farfisa-proc-gain.csv](data/farfisa-proc-gain.csv) (sum, Hz, dB, spread). Rust: `organ/farfisa_proc_gain.rs` (`farfisa_proc_gain(sum, hz)`, test against the CSV), **not wired into the render path**.

| Check | Result |
|---|---|
| Linearity (weight 1.0 against 0.5, divided by the weight) | max difference 0.00 dB over 110 bins (VERIFIED) |
| Note independence | median spread of the slots inside one 1/6 octave bin 0.9 dB (it includes the slope of G inside the bin), so G depends on frequency, not on the note (EMPIRICAL) |
| Sums | sum 0 reads ring slot `idx`, sum 1 slot `idx + 8`; a key therefore feeds the 16' row (`note - 12`) through sum 0 and the 8' row (`note`) through sum 1 (the third key word, sum 2, and sum 3 give output 0.0 exactly with the installed gains, also with weight 1.0 at the key's own index) |
| Shape of G | sum 0: -1 dB at 60 Hz, -6 dB near 400 Hz, +1.2 dB at 900 Hz, then falling to -26 dB at 8.6 kHz; sum 1 has a similar shape (see the table) |
| Output scale | gain per unit weight is O(1): a weight of 1.0 gives rms 0.5; the key weight 1/48 gives the chord peaks of s7 |

Not done (time box): per-line comparison of a Rust render with the oracle chords (needs the Farfisa key to bus mapping with two filters per wheel: a wheel is the 8' row of key `n` through sum 1 and the 16' row of key `n + 12` through sum 0, so the bus-of-wheels path cannot apply one table per wheel; the filter has to act on the key mix or on two wheel copies), the B3 Bass table (its weights come from the ARM key-down `DM 0x951F1..` over two ring windows, not the Farfisa words), `emu_vs_hw.py` with `--organ-level firmware`, and the video check. Defaults and `render_hash` unchanged; the opt-in sources stay opt-in because the oracle criterion is not met (no processor in the render path) and the video check was not run.

## 9. Wiring of the Farfisa key stage and the B3 Bass status (2026-11-14)

Wired: `organ/farfisa_keys.rs`, opt-in `--farfisa-processor on` ([organ-engine](../specs/organ-engine.md) s3.1.8). Per key `n` the firmware key-down writes `1/48` into sum 0 (ring note `n - 12`) and sum 1 (note `n`); the stage multiplies the two wheels with the measured gains of s8 (two linear-phase FIRs). Check (`cargo test -p nord-core --lib farfisa_keys`): chord a, keys 60 64 67, generator `FarfisaWheels` at 48 kHz, 1 s Hann Goertzel at the note frequencies: C4 -43.0, G4 -43.8, C3 -45.9, E3 -45.7 dB (oracle 0, 0, 0, -1 dB re the strongest), peak 0.08835 against 0.08666 (+0.17 dB). Open: the 16' rows are about 2.5 dB lower than in the oracle (phase and the lattice are not modelled; the table is a median over 1/6 octave bins), the tab selection and the vibrato.

B3 Bass: **not ported.** What is known for a port: the oracle chords of s7 show only the 8' partial of each key (C4 E4 G4 at 0, -2, -2 dB, nothing at 16' within 38 dB), so the Bass processor reads the ring at note `n` and the note an octave above with weights that the ARM key-down writes at `DM 0x951F1..` (`keydown-organ-models.csv`, model 5, notes 36, 60, 84, index words at `DM 0xB4246..`); the sums feed the 33-tap FIR, two lattice groups, the 9-tap scanner delay and three biquads (s4.2). A line-gain stage like the Farfisa one needs the same `sharc-chain` measurement for the Bass record (`0xB472C`) on the B3 generator ring; it was not made, and the per-key weight words of model 5 were not decoded into a closed form. Until then `b3_bass_mode` keeps its ASSUMED pickup table ([organ-engine](../specs/organ-engine.md) s3.1.1).

## 10. B3 Bass key-down weights for every key, and why there is no line-gain table (2026-11-14)

Tools: `tools/organ_bass_keys.py capture` (the real ARM key-down of `Edt_Org_kModel = 5`, drawbars `888888888`, velocity 100, keys 24..108, one emulator session; data [bass-keydown.csv](data/bass-keydown.csv) = `key,window,index,value_bits`), `tools/organ_bass_proc.py chain` / `measure` (`sharc-chain`: generator `0x121BBE` + processor `0x122296` per call, ring slots of both frames at `+0x68`).

| Finding | Result | Tag |
|---|---|---|
| Weight windows (listing `0x122296`) | window 1 `DM 0x951F1 + k` (56 words, ring slot `0xB32E8 + k`), window 2 `DM 0x95241 + k` (68 words, ring slot `0xB32F4 + k`), both added | VERIFIED (listing, `adi_dis`) |
| Closed form of the key-down | the **percussion item** writes one word per key into window 2 at index `n - 24` (keys 24..91, 68 of 68 keys; keys 92..108 none, key 103 folds onto index 67); value = per-key taper table (0.00908 up to key 36, 0.0166 at key 66, 0.0116 at key 84), `organ/b3_bass_keys.rs` | VERIFIED (ARM run, test `table_matches_the_derived_csv`) |
| Window 1 (drawbar item, footages 16' / 5 1/3' / 8' at partial note `- 36`) | inaudible: with window 1 zeroed the chord `60 64 67` is bit-identical (`chain --zero 1`); with window 2 zeroed the output is 0 | VERIFIED (run) |
| Chord goldens of the oracle (`sharc-chain`, 8192-point Hann, line amplitude) | keys 60, 64, 67: 0.02548, 0.02449, 0.02050; the same keys alone: 0.0259, 0.02491, 0.02565 (-0.15, -0.15, -1.95 dB for the chord) | VERIFIED (run) |
| 2nd harmonic of the output | -38 dB below the fundamental (ring harmonic -15.7 dB): the processor attenuates harmonics | VERIFIED (run) |

**Why the Farfisa recipe (`G(f)` per weight window, `farfisa-proc-gain`) does not carry over.** The gain `|Y| / |R|` of one weight word at 1.0 and one ring slot is **state dependent**: for ring note 60 it is +16.1 dB after the chord script (3 keys) and +2.3 dB after the 85-key script (`measure`; linear in the weight in both). The difference is not the weight windows: the key-down also writes the lattice and filter words `DM 0x954E6..0x95529` (117 words differ between the two scripts: pointers `0xB33xx` into the generator ring, `0xB45xx` into the delay line, coefficients about -0.004 .. -0.024), i.e. per-key resonator sections (`arm_model_paths.py`: "resonator B") that the ARM allocates in key order. The output of a chord therefore depends on the slot a key lands in and on the A words that several keys share (C4 E4 G4 gives G4 -1.95 dB below its single-key level; C4's 5 1/3' partial rewrites the A word of note 67, ASSUMED cause).

Consequence: a Rust port that meets the 1 dB line criterion against the oracle needs the per-key section words (`0x954E6..`, `0x955B0..`) and the A-word accumulation rule, not a per-sum gain table. The weight table of this section is exact and committed (`b3_bass_keys.rs`) but **not wired**: `b3_bass_mode` keeps its ASSUMED pickup table, defaults and `render_hash` are unchanged, and no hardware clip comparison (`emu_vs_hw.py`) was made.

Follow-ups (superseded by s11: the section words are not the cause; s12 closes the ring weights): (1) decode the section words the key-down writes per key (slot allocation by key order, pointer pairs, coefficient closed form) from the same ARM capture; (2) the A-word accumulation (several footages and keys onto one ring note, `keydown-zones.md` s6.2); (3) then the per-key response, the Rust stage and chord goldens within 1 dB.

## 11. B3 Bass state: the "section words" are not the cause, `G` is (2026-11-14)

Tools: `tools/organ_bass_state.py capture` (the ARM key-down for keys 24..108, [bass-state.csv](data/bass-state.csv) = per-key increments of window 1 / 2 words and the step of `S`), `tools/organ_bass_proc.py measure` (now with `G` pinned to 0; [bass-proc-gain.csv](data/bass-proc-gain.csv)), `sharc-chain` chords with the ring recorded (goldens [bass-chord-goldens.csv](data/bass-chord-goldens.csv): 1, 3 and 10 keys, 3 key orders). Rust: `organ/b3_bass_state.rs` (not wired).

| Finding | Result | Tag |
|---|---|---|
| Section words `DM 0x954E6..0x9552F`, `0x9559C..0x955AF`, `0xB4246..` | Not the cause of the state dependence: zeroing `0x954E6..0x95530` changes the chord output by 4 % rms (0.3 dB), `0x9559C..` and `0xB4246..` by 2 %. They are per-key voice records (5 pair groups per key, a second key takes `0x95518..0x95549`, later keys rewrite group 0); the processor listing never addresses them. Correction of s10 | VERIFIED (run, `target` scripts of the commands below) |
| What flips the gain by 13.4 dB (3-key against 85-key script, ring slot 48) | One word, `DM 0xB44C9` (`G`): copying only that word of the 85-key state into the 3-key state reproduces it (-2.70 to -16.13 dB); `0xB44CA`, `0xB44CB` do not matter | VERIFIED (bisect over the 281 differing words) |
| Output law | output = `(1 - 0.84 G)` x linear part, to 0.01 at G = 0.2 .. 0.95 and at three ring slots (sweep of the word, G = 0.4 is -3.55 dB) | VERIFIED (run) |
| Accumulation of `G` | `G = phi(S)`, `S` = plain sum of per-key steps `inc0(key)` (0.0257 for keys 24..36, 0.0262 / 0.0258 / 0.0261 for 60 / 64 / 67), `phi` piecewise linear with slope `2^-n` on `S` in `[n/2, (n+1)/2)` (the ARM halves the step above `G` = 0.5, quarters it above 0.75, ...; a step that crosses a breakpoint is split, key 43 of the ascending run). Order independent. Six replays (1, 3 keys both orders, 10 keys, 85 keys): max error 5e-7 | VERIFIED (run), steps of keys 24..108 derived from one ascending run |
| Weight windows | **Additive** over the held keys, increments per key in `bass-state.csv` (reproduces every window word of 5 replays exactly). Window 1 is **audible** (zeroing it leaves 36 % of the output rms in the 3-key script; s10 claimed bit-identical, that held only for its script) | VERIFIED (run) |
| Generator weights `DM 0x95122..` (the ring is `wheel x weight`) | **State dependent and not additive**: a wheel written by a second key keeps a smaller value (0.5465 to 0.4367, 0.3532 to 0.2436, ratios 0.69 to 0.82); the ring amplitude of slot 48 is 0.262 after 3 keys and 0.061 after 85. The rule is not decoded, so the full stage cannot be built | VERIFIED (run), rule OPEN |
| Processor line gain at G = 0 | Per window a smooth function of the line frequency (bin spread 0.0 to 1.5 dB): window 1 +8.4 dB at 74 Hz, -5.9 at 260 Hz, -29.6 at 1.2 kHz; window 2 +9.1, -2.1, -15.6 | EMPIRICAL (1/6 octave bins) |
| Processor part against the oracle | `line = ring_amp x (W1 g1(f) + W2 g2(f)) x (1 - 0.84 G)` with the oracle's own ring line amplitudes: 45 lines of 6 chords (1, 3 and 10 keys, press orders ascending, descending, shuffled), max 1.45 dB, rms 0.4 to 0.65 dB, 42 of 45 within 1 dB; coherent sum of the two windows (power sum: 3.1 dB) | VERIFIED (run, `b3_bass_state` test `chord_goldens_match_the_oracle_lines_within_1p5_db`) |

```bash
python tools/ns4_dsp_images.py target/fw_unpacked/bank0_CODE.bin --out target/DSP
python tools/ns4_dsp_ldr.py target/DSP/dsp_image_1_PNO.bin --sram target/PNO
uv run --with unicorn --with capstone --with numpy python tools/organ_bass_state.py capture     target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin docs/research/data/bass-state.csv
python tools/organ_bass_proc.py measure target/PNO/core1_sram.bin <script> out.csv --exe target/release/sharc-chain
cargo test -p nord-core --lib b3_bass_state
```

**Not met: the <= 1 dB criterion on a full Bass stage.** The processor half (weights, `G`, line gain) is exact to 1.45 dB given the ring; the ring needs the generator weight rule above. Nothing is wired, `render_hash` is unchanged, and `emu_vs_hw.py` was not run (a stage that is not in the render path cannot change the clip; the bass band delta is therefore 0.00 dB).

## 12. B3 Bass: generator weight rule, key-up, and the wired opt-in stage (2026-11-14)

Tools: `tools/organ_bass_gen.py capture` (ARM key-down of keys 24..108, drawbars `888888888`; data [bass-gen-weights.csv](data/bass-gen-weights.csv) = table `T` and writes per wheel and key), `cap` scripts for the real MIDI path (`arm_zone_capture.prepare` + note-on / note-off, local only). Rust: `organ/b3_bass_gen.rs` (rule), `organ/b3_bass_stage.rs` (stage), `organ/gain_fir.rs` (FIR design shared with the Farfisa stage).

| Finding | Result | Tag |
|---|---|---|
| Rule of the generator weights `DM 0x95122 + (note - 36)` | The word of a wheel is a **pure function of the number `n` of earlier writes to that wheel still held**: `weight(note, n) = T[note][n]`, `T[note][0]` = the first-write amplitude `A`. Not additive; not dependent on the key, the footage item (drawbar or percussion), the press order or the drawbar levels (a zero drawbar still writes). 297 of 297 writes of 9 sequences (single key, 60 + 60, 60 + 72, 60 + 67, 67 + 60, 60 64 67, 8 keys, all 85 keys) match `T` bit for bit | VERIFIED (run, `tools/organ_bass_gen.py`, test `chord_words_match_the_arm_key_down`) |
| Size of the table | 79 wheels (notes 36..114), up to 27 writes per wheel at the top of the keyboard; the words of notes 12..35 stay 0 in the image, so ring slots 0..23 are silent | VERIFIED (image, run) |
| Shape of `T[note]` | falls monotonically; ratio `T[1]/T[0]` is 0.78 to 0.82 on wheels 60 to 96 and 0.94 to 0.96 on wheels 36 and 48, the ratios of one wheel approach 1 (wheel 79: 0.785, 0.820, 0.841, ...). No closed form found (reciprocal, power and exponential forms leave 0.3 to 1.3 % residual); the table is committed | VERIFIED (table), law OPEN |
| Key-up (real MIDI path, notes 60 64 67, key-up of 64) | The key's writes are removed one by one and the word becomes `T[note][held - 1]` (wheel 88 back to 0.54649, wheel 83 passes 0.37065 and ends at 0.46984, wheel 76 with no other holder returns to `T[0]` = 0.39976). Window words go back to the sum of the remaining keys (the burst of tick 117 rewrites them with float residues of 2e-5). `G` first drops by the step of the key (0.01346 to 0.00897) and then an idle rebuild sets 0.01071, 19 % above the replay (0.01 dB of output scale) | VERIFIED (one run), rebuild ASSUMED to be a resync |
| Real MIDI path against the direct item call | Same weights per wheel (first writes and the table agree), but the real path uses the default program's drawbars (8 footages written for key 60, `G` steps 0.0045 instead of 0.0262, 5 1/3' of key 64 at wheel 59). The key tables of s10 / s11 are the `888888888` direct call; the stage uses them for every drawbar setting | VERIFIED (run), consequence ASSUMED |
| Ring check | wheel 60 after 85 keys is 0.2313 of its first write (11 writes); with 0.262 after 3 keys this gives the 0.061 of s11 | VERIFIED (test `all_keys_pull_the_weights_down_by_the_arm_table`) |

**Stage** (`--b3-bass-processor on`, `OrganLayer::set_b3_bass_processor`): `ring[s] = wheel(note s + 12) * T`, `x1 = sum W1[k] ring[k]`, `x2 = sum W2[k] ring[12 + k]`, two 511-tap linear-phase FIRs from the measured window gains (`bass-proc-gain.csv`), output `* (1 - 0.84 G)`, folded into one coefficient per wheel and window and ramped over 2 ms. It needs `ToneWheelSource::B3Wheels` on a B3 Bass layer; turning it on selects the wheel tables without the B3 tail.

| Check | Result |
|---|---|
| Chord 60 64 67 against `bass-chord-goldens.csv` rows `g3` (6 lines, 1 s render at 48 kHz, Goertzel at the stage frequencies, `cargo test -p nord-core --lib chord_lines_follow_the_oracle_goldens`) | +1.06, +1.42, +1.55, +2.50, +1.67, +1.57 dB (stage louder). Mean +1.6 dB: the oracle values are 8192-point Hann lines and read up to 1.4 dB low (scalloping, the ring line of slot 48 reads 0.262 where `A x table` is 0.289, -0.85 dB); the rest is the magnitude-only FIR and the project tuning. **Not within the 1 dB bar** for slot 48 (2.5 dB) |
| Key-up, octave shift, ring-down | tests `release_rings_down_to_silence_and_stays_finite`, `octave_shift_registers_the_transposed_key_and_releases_it` |
| Default render | `render_hash` unchanged (flag off by default, no code path changes when off) |

**Clip comparison and default (D22, D41).** The B3 clip of [organ-presence-gap](organ-presence-gap.md) (`B3 Chords n Solo`) plays the **B3** model, not B3 Bass: the stage requires `b3_bass_mode`, so the bass band delta of `emu_vs_hw.py` with and without `--b3-bass-processor on` is **0.00 dB by construction**; no hardware clip of a B3 Bass program is available, and the chord check above is 1.1 to 2.5 dB. No default is flipped.

Still open: the stage does not model the drawbar and percussion settings (the key tables are the `888888888` key-down; the real default program writes smaller window steps, so levels differ), the bit-exact `0x122296` model (with the 1.45 dB residual of the processor part), the scanner inside the processor, the closed form of `T`, the `G` resync after a key-up, and a B3 Bass hardware clip.

## 13. Instruction-level model of `0x122296`: the listing runs bit-exact in Python (2026-11-14)

Tools: `tools/sharc_listing_vm.py` (interpreter of the ADI listing text, SIMD, 40-bit float), `tools/organ_listing_check.py` (trace and memory comparison with `sharc-chain`), tests `tools/tests/test_sharc_listing_vm.py`. The listing is read locally from the image (`tools/adi_dis.py`); nothing of it is committed.

```bash
python tools/adi_dis.py target/PNO/core1_sram.bin 0x24452C 0x244C40      # listing, byte = 2 pc
python tools/organ_listing_check.py trace target/PNO/core1_sram.bin bass_m5.script \
    --entry 0x122296 --end 0x12261d \
    --gen 0x121bbe:0x121e28:I0=0xc00f2,I1=0xb52c7 \
    --proc 0x122296:0x12261d:I0=0xc00ea,I1=0xb472c --exe target/release/sharc-chain
```

| Check | Result | Tag |
|---|---|---|
| Both frames of call 0 (boot + model 5 install + key-down 60 64 67), every instruction, all R, S, I, M words after it | 868 of 868 rows, 0 mismatches | VERIFIED (run) |
| Memory after the call (weights, state blocks, delay line, output stream: 16,128 words) | 0 words differ | VERIFIED (run) |
| Float format | 40 bit: the 8 extension bits survive between operations (moves and swaps keep them, loads clear them, stores write the upper 32); a single precision model is 1 ulp off after about 70 instructions | VERIFIED (trace) |
| SIMD data access | a data register load or store moves the PEy twin to `addr + 1`; loads through `I1` broadcast the word at `addr` to both elements (MODE1 BDCST1; the state block is read through `I1`); stores always use the pair; `Rx <-> Sy` is a single exchange | VERIFIED (trace) |
| Structure | per call 2 frames; per frame: the window 1 and window 2 dot products as two lanes (PEx even, PEy odd words; 28 and 34 products per lane, two accumulators per lane), the lane fold (`r2 = s8; f0 = f8 + f2`), two param-block reads of the scanner, the 33-tap FIR and two lattice groups on the circular buffer `l6 = 0x21`, 9 delay taps weighted by `DM 0xC0202..`, three biquads, the DC blocker, the output store | VERIFIED (listing run) |

Limits. (1) One call: the generator `0x121BBE` also writes processor state words between calls (`DM 0x9559C..0x955AF`, `0xB45AE..`, `0xC0209`, found by diffing two generator-only runs), so a multi-call replay needs the generator listing in the interpreter (a model started from the oracle ring diverges from call 1). (2) The interpreter has no branches or flags: the Farfisa processor `0x122B85` stops at `if ne jump 0x122d88`. (3) The clean transliteration (a hand written recursion in Rust for the opt-in Bass stage) is not done: the part after the lane fold uses the PEy element as a helper (lane swaps `r13 <-> s14`, `s2 = r3`, a delay line shifted through `I0 / I1`), so it needs a derivation per section, not a copy of the listing. The Rust stage therefore keeps the EMPIRICAL magnitude FIR of s12 (+1.1 to +2.5 dB on the 60 64 67 goldens, unchanged) and the Farfisa stage its measured gain tables. Not decoded in this pass: the drawbar and percussion dependence of the key tables, the `G` resync after a key-up, Twin amp types 7 to 9, `0x122B1D`, the cabinet biquad bank.

## 14. Generator in the VM and the multi-call replay: the chain runs bit-exact over whole chord scripts (2026-11-14)

Tools: `tools/sharc_listing_ext.py` (`GenVm`: the generator instruction set on top of `sharc_listing_vm.py`), `tools/organ_replay.py` (`gentrace`, `replay`), tests `tools/tests/test_sharc_listing_ext.py`. Closes limit (1) of s13.

```bash
python tools/organ_replay.py gentrace target/PNO/core1_sram.bin bass_m5.script \
    --gen 0x121bbe:0x121e28:I0=0xc00f2,I1=0xb52c7 --exe target/release/sharc-chain
python tools/organ_replay.py replay target/PNO/core1_sram.bin bass_m5.script --calls 4696 \
    --gen 0x121bbe:0x121e28:I0=0xc00f2,I1=0xb52c7 --proc 0x122296:0x12261d:I0=0xc00ea,I1=0xb472c \
    --exe target/release/sharc-chain
```

Scripts: `tools/organ_fb_pass.py script ... --model 5 --keys K --drawbars D --cache target/settled.snap` (boot + model 5 install + key-down of the keys, as in s7).

| Check | Result | Tag |
|---|---|---|
| Generator `0x121BBE`, call 0 (every instruction against `sharc-chain --trace`) | 1432 of 1432 rows, 0 register mismatches; 0 memory words differ after the generator | VERIFIED (run) |
| Multi-call replay (generator then processor, per call, presets `I0 / I1` as the oracle), windows compared per call: the output stream `DM 0xC00EA..` (4 words) and the ring `DM 0xB32E8..` (0xD0 words); memory (16,128 words in four ranges) compared after the last call | keys 60 64 67, drawbars 888000000: **4696 calls** (the length of the 8192-point chord measurement of s11), 0 differing calls, 0 differing words; keys 36 43 48 52 55 60 64 67 72 76 (drawbars 888888888), keys 60 64 67 with drawbars 888888888 and with 008000000: 1200 calls each, 0 differing calls, 0 differing words | VERIFIED (run) |
| What the generator writes between calls | everything is reproduced by the listing (processor state words `DM 0x9559C..0x955AF`, `0xB45AE..`, `0xC0209`, the ring, the scanner position words, the `i12 / i13` long word reload of `0xB32D6`): no hand written state is needed | VERIFIED (run) |
| Speed | about 30 ms per call (generator + processor, about 1,900 instructions) in CPython | measured |

New interpreter rules (each checked by the trace, nothing assumed): `fext / fdep Rx by Ry` take the length from `Ry[11:6]` and the position from `Ry[5:0]` (`0x1D9` = 7 bits at 25; the text form `by 0x19:0x7` is `pos:len`); `float Rx by Ry` converts the integer with the exponent offset `Ry` (signed); `clip Rx by Ry` clamps to `+-|Ry|`; fixed multiplies `(ssf / ssfr / ssi)` with `MRF` as source or destination, one `MRF` per PE (the `MR` of PEy is separate); `comp(Rx, Ry)` sets `AN / AZ` per PE and `if lt` executes per PE; a store of an index or modify register (`dm(a)=i2`) writes the PEy twin to `a + 1` as well; `dm(a)=i12 (LW)` stores the pair `i12, i13`.

**Time variance of the processor (EMPIRICAL, one-off run with the replayed state).** With the ring zero, all weights zero except one word and the zero input response subtracted (the delay line and biquads hold the chord's state), the output to an impulse on one ring slot is linear (scale 2.0 gives 2.0 x the response to 1.4e-5 of the peak) and does not depend on the slot (slots 0 and 10 of window 1, 0 and 5 of window 2 give the same first samples and the same energy to 16 digits), but it is **not time invariant**: the same impulse 7 calls later differs from the shifted response by 25 % of the peak, because the delay taps that the scanner reads (`m3 = dm(0x10, i2)` ... and the circular buffer on `l6`) move from sample to sample. Consequence: no fixed FIR (the EMPIRICAL magnitude FIR of s12 included) can reproduce the processor; the line gains of s11 are averages over the scanner phase (bin spread up to 1.5 dB). The Rust recursion has to carry the scanner positions (`DM 0xB33BA..0xB33C0`, the per sample delay offsets that the generator writes) and the lattice sections.

Not done in this pass (and why): (2) the Rust recursion and the oracle re-check (the part after the lane fold is a software pipelined sequence of about 280 instructions with PEy helpers; it needs the scanner phase law first, the Rust stage keeps the EMPIRICAL FIR, +1.1 to +2.5 dB on the chord goldens, unchanged); drawbar / percussion key tables and the `G` resync after a key-up (need the ARM capture with key-up, only the direct drawbar script is replayed here); (3) branches and flags for `0x122B85` (conditional execution and `comp` now exist, branches and a Farfisa model 2 install with weights are still missing), so the Farfisa gain-table FIRs stay; (4) Twin amp types 7 to 9, `0x122B1D` and the cabinet biquad bank.

## 15. Firmware recursion in Rust: the Bass stage runs the generator and processor at the DSP rate (2026-11-14)

Tools: `tools/sharc_listing_rust.py` (decodes the listing with the rules of the Python VM into integer rows), `tools/organ_bass_fw_gen.py` (`image` / `convert` write the install delta, `delta` / `golden` from `sharc-chain`; the `rust` tables are gone, D128), `tools/organ_bass_voice.py` (ARM capture of the voice records, `table`). Rust: `organ/b3_bass_fw/` (`f40.rs` 40 bit float and fixed multiplier, `exec.rs` instruction executor, `mod.rs` machine, `image.rs` rows and DM image decoded at run time from the user's OS update file; the former GENERATED `prog_gen.rs` / `prog_proc.rs` are deleted), `organ/b3_bass_voice.rs`, `organ/b3_bass_stage.rs`. Data: [bass-fw-install-delta.csv](data/bass-fw-install-delta.csv) (12 start registers and the 4,732 DM words of the model 5 install that the loader data lacks; the full 6,881 word image of the installed model 5 is rebuilt from the user's file), [bass-fw-chord-delta.csv](data/bass-fw-chord-delta.csv), [bass-fw-chord-golden.csv](data/bass-fw-chord-golden.csv), [bass-voice-slots.csv](data/bass-voice-slots.csv).

```bash
python tools/organ_bass_fw_gen.py image target/PNO/core1_sram.bin n888.script bass-fw-install-delta.csv --code target/fw_unpacked/bank0_CODE.bin --exe target/release/sharc-chain
python tools/organ_bass_fw_gen.py delta target/PNO/core1_sram.bin n888.script bass-fw-chord-delta.csv --script2 c3.script --exe target/release/sharc-chain
python tools/organ_bass_fw_gen.py golden target/PNO/core1_sram.bin c3.script bass-fw-chord-golden.csv --calls 48 --exe target/release/sharc-chain
python tools/organ_bass_voice.py capture target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin cap.csv --keys 24:28 --drawbars 888888888
cargo test -p nord-core --lib b3_bass
```

(`n888.script` = `tools/organ_fb_pass.py script ... --model 5 --keys= --drawbars 888888888`, `c3.script` the same with `--keys 60,64,67`.)

Design. The scanner phase law, the lattice sections and the 40 bit float behaviour are not re-derived by hand: the two routines are decoded once into rows (`INS` / `LOOP` plus one row per instruction item, 898 rows for `0x122296`, 555 for `0x121BBE`, no firmware bytes) and executed by a small machine in Rust with the rules of the Python interpreter. Single precision was **not** used: s13 measured 1 ulp of drift after about 70 instructions, and the 40 bit executor is bit exact.

| Check | Result | Tag |
|---|---|---|
| Rust machine against `sharc-chain` (boot + model 5 install + key-down 60 64 67, drawbars 888888888): output words of 48 calls (generator + processor), test `decoded_chain_is_bit_exact_against_the_oracle_golden` | 48 of 48 calls, both frames, every word equal | VERIFIED (run) |
| Executor rules (post modify and broadcast loads, long word stores, per PE `if lt`, `comp`, circular buffers) and the 40 bit arithmetic | unit tests in `exec.rs` / `f40.rs`; the Python rules were checked against the oracle in s13 / s14 | VERIFIED |
| Generated rows are current | `python tools/organ_bass_fw_gen.py rust ... --check` | VERIFIED |
| **Line criterion** (<= 1 dB) of the stage against `bass-chord-goldens.csv`, all 46 lines of the six chords g1, g3, g3r, g10, g10r, g10s (8192 point Hann peak bin at the DSP rate, 1,200 samples skipped; test `single_key_and_chord_lines_follow_the_oracle_goldens`) | worst line error 0.0036 dB (g1), 0.0005 dB (g3, g3r), 0.0011 dB (g10, g10r, g10s). The EMPIRICAL FIR of s12 read +1.1 to +2.5 dB on g3 | VERIFIED (run), state words per key ASSUMED where listed below |

Scanner phase law (read from the generator tail, not separately measured). The state block is at `DM 0xC01F6` (`i7`; the long word at `0xC01F6` loads `i6 = 0xC0262`, the 128 entry table centred at `i6`, and `i7`): words `+2 / +3` the increment `0x002A454D` per lane, `+4 / +5` and `+6 / +7` two 32 bit phases (start `0x40000000` and `0xC0000000`), `+8 .. +11` wrap counters. Per call both phases advance by `2 inc`; a wrap (`comp` of the new against the old phase, `if lt`) steps the counters by 8 modulo 32. The index is `fext phase by 25:7 (se)` (7 bit signed, `-64..63`), the fraction is the low 25 bits (`fdep ... 6:25`, `float` by `-31`); the tap is the linear interpolation `(1 - f) T[i] + f T[i + 1]` of the table (values 0 to 1.07, a smooth bump) stored at `0xC0202 + m`, which the processor reads with `m3 = dm(0x10 .. 0x18, i2)` (nine taps on the delay line `0xB46E6`); the frame loop steps `0xB33BC` by 2 per frame (reset every call). The 28 and 34 product lane dot products read the ring at `0xB32E8 + dm(0xB33BA)` and `0xB32F4 + ...` (`0xB33BA` + 0x68 per frame), the lane fold is `f0 = f8 + s8`, the 33 tap FIR and the two lattice groups run on the circular buffer `l6 = 0x21` (`pm(i11)` pointers), then the nine tap sum, three biquads, the DC blocker and the output store. All of it is executed, not approximated.

Key-down state fed to the machine by the stage (`b3_bass_stage.rs`):

| Words | Source | Tag |
|---|---|---|
| Windows `DM 0x951F1 + k`, `0x95241 + k` | `BassState` (s11), additive per key | VERIFIED (exact for 888888888) |
| Generator weights `DM 0x95122 + (note - 36)` | `GenWeights` `T[note][n]` (s12) | VERIFIED |
| `G` `DM 0xB44C9` | `phi(S)` (s11) | VERIFIED |
| Voice records `DM 0x954E6 + 20 j`, `0x9559C + 4 j`, `0xB4246 + 12 j` (slot `j` = press mod 5) | `bass-voice-slots.csv`: per key, the 36 words of the ARM key-down at its slot of a fresh session, relocated | key dependence VERIFIED (capture); the per press jitter (pointer offsets, coefficient scatter, drawn by the ARM) ASSUMED: the same key at another slot has other words, and the line levels do not depend on them (above) |
| `DM 0xB44CB` (a second accumulator, about 0.03 per key) | not written | ASSUMED irrelevant (zeroing `0xB44C9` and `0xB44CB` of the 3 key state changes the output by 2.5 % rms, almost all `G`) |

The voice records are the part that s11 called "not read by the processor": the generator **does** read them (ten sections per call over `0x954E6..0x95549`, `0x9559C..`, `0xB4246..`). Restoring the nokeys values of the coefficients, the shift counts or the masks each leaves only 3 % to 31 % of the output rms, so the installed values matter; but the chord line levels are reproduced to 0.004 dB with the jitter of other presses, i.e. the jitter does not change the line amplitudes. They are a per-key table here, not a closed form. What the capture shows: the drawbar item writes 3 sections and the percussion item 1 per key (888000000), the first five presses fill slots 0..4 (20 + 4 + 12 words), the 6th and later presses rewrite slot 0, pointer pairs rotate through a 7 entry list (`0xB3424 .. 0xB35EA`) with the press count, and the coefficient (about `-0.008` at key 24, `-0.022` at key 108) rises with the key.

Cost. Release build: 3.6 s of CPU per second of audio (about 21.7 k calls per second, 70 ns per decoded instruction): the stage is for **offline rendering** (`ns4 render`, `emu_vs_hw.py`) and not real time. Faster paths (a single precision or fixed point recursion with the same rows, a Rust code generator from the rows) are open. The stage does no work with no key held after a 10,000 call ring-down (ASSUMED).

Defaults. `--b3-bass-processor on` stays opt-in (D22): the key tables are the `888888888` key-down (the real default program writes smaller window steps), the `G` resync after a key-up is replayed (0.01 dB), and no B3 Bass hardware clip exists (the B3 clip of [organ-presence-gap](organ-presence-gap.md) plays the B3 model, so the `emu_vs_hw.py` bass band delta is 0.00 dB by construction). `render_hash` is unchanged.

Not done (time box; the Farfisa part and the real time path are s16 / s17): drawbar and percussion dependence of the key tables (only `888000000` and `888888888` were captured), key-up capture and the `G` resync, a closed form of the voice jitter, `tools/emu_vs_hw.py` on a Bass clip (none exists).

## 16. Farfisa processor `0x122B85` on the firmware machine (2026-11-15)

Tools: `tools/organ_farfisa_fw.py` (`check`, `image`, `golden`, `chords`; the `rust` command was removed, D128), `tools/sharc_listing_vm.py` / `sharc_listing_ext.py` (jumps, AZ / AN flags), `tools/sharc_listing_rust.py` (jump rows, the reference of `organ/farfisa_fw/rows.rs`). Rust: `organ/farfisa_fw/` (rows and DM image built at run time from the user's OS update file since 2026-11-23: `image.rs`, `rows.rs`, D128; the tracked `prog_proc.rs`, `farfisa-fw-image.csv` and the straight line `native_farfisa.rs` are gone), `organ/farfisa_keys.rs` (the stage). Data: [farfisa-fw-image.csv](data/farfisa-fw-image.csv) (model 2 install, 802 words of the ranges the routine touches, start registers), [farfisa-fw-golden.csv](data/farfisa-fw-golden.csv) (24 calls, chord 60 64 67: output words and the generator ring of each call), [farfisa-fw-golden-flag1.csv](data/farfisa-fw-golden-flag1.csv) (12 calls, second lattice table), [farfisa-chord-goldens.csv](data/farfisa-chord-goldens.csv) (the 12 strongest 8192 point Hann lines of four oracle chords).

```bash
uv run --with unicorn --with capstone --with numpy python tools/organ_proc_oracle.py   # boot log (no sentinels) + 526 install words = m2.script
python tools/organ_farfisa_fw.py check  target/PNO/core1_sram.bin f2_k3.script --calls 300 --exe target/release/sharc-chain
python tools/organ_farfisa_fw.py image  target/PNO/core1_sram.bin m2.script docs/research/data/farfisa-fw-install-delta.csv --exe ...
python tools/organ_farfisa_fw.py golden target/PNO/core1_sram.bin f2_k3.script docs/research/data/farfisa-fw-golden.csv --calls 24 --exe ...
python tools/organ_farfisa_fw.py chords target/PNO/core1_sram.bin --image-out docs/research/data/farfisa-chord-goldens.csv \
    --chord k1:60:f2_k1.script --chord k3:60,64,67:f2_k3.script --chord k48:48,55,64:f2_k48.script --chord k72:72,76,79:f2_k72.script --exe ...
cargo test -p nord-core --lib farfisa
```

(`mK.script` = the ARM boot log `target/bulk/pno_core1.script` without the two sentinel words, then the 526 words of the model 2 install; `f2_kN.script` = that plus `organ_proc_oracle.farfisa_keydown(keys)`: weight `1/48` at `DM 0x95FE4 + key + {0, 0x62, 0xD0}` and the three bias words. `organ_fb_pass.py script --model 2` alone gives a silent pass: it replays the key-down items of the B3 sub-engines, which write nothing for model 2.)

What the listing needed beyond s13 / s14: one conditional jump `if ne jump 0x122d88` (picks the lattice table `pm(i11 - 0x1E)` or `pm(i11 - 0x1C)`), one `jump 0x122e0a`, both forward and inside the frame loop, and the ALU flags. The jump condition is the PEx `AZ` of the instruction before it, `f1 = pass f1`, where `f1` is the state word `DM(i3 - 0x14)` = `DM 0xC2C2C` (0 in the image: the first table). The ALU sets `AZ` (result exponent 0 for a float, word 0 for a fixed op) and `AN` (sign) for pass / add / sub / abs and the fixed add / sub / clip / and; the multiplier and the shifter leave them alone (VERIFIED: 300 calls bit exact in both branches; a flag set by a PE that skipped an `if lt` instruction is not updated). The row format gains `INS n cond jcond target` (a jump is an `INS` with no items, the target an absolute row index); the Rust machine continues at that row; hardware loops and jumps do not mix with delay slots (none are used).

| Check | Result | Tag |
|---|---|---|
| Python VM against `sharc-chain`, processor only, the oracle's own ring injected per call (`organ_farfisa_fw.py check`): chord 60 64 67, 300 calls, flag 0 and flag 1 (`DM 0xC2C2C` poked), 200 calls each | 0 calls with a differing output word (flag 0 and 1 give different outputs from call 0, so both tables run) | VERIFIED (run) |
| Rust machine on the same words (`farfisa_fw::tests::decoded_processor_is_bit_exact_against_the_oracle_golden`, `second_lattice_table_is_bit_exact_too`) | 24 and 12 calls, both output words of each call equal | VERIFIED (run) |
| Generator `0x122A71` | **not** run on the machine: it needs `float` clip, `max`, `fix` and a register loop count (`lcntr=r3`). The ring comes from `FarfisaWheels` at the DSP rate (frame 0 at `0xC2B52`, frame 1 at `0xC2BB4`, slot `12 r + n`), which agrees with the generator to 2e-4 (`farfisa_wheels::golden_rows_match_the_python_model`) | ASSUMED equal for the line check below |
| **Line criterion** (<= 1 dB, target 0.01 dB) of the stage on the 12 strongest lines of each oracle chord (`farfisa_keys_tests::chord_lines_follow_the_oracle_goldens`, 8192 point Hann peak bin at the DSP rate, 1,200 samples skipped): chord 60 / 60 64 67 / 48 55 64 / 72 76 79 | worst 0.0039 / 0.0006 / 0.0008 / 0.0010 dB (mean 0.0014 / 0.0002 / 0.0003 / 0.0004 dB). The gain-table FIR stage of s9 was within 2.9 dB (EMPIRICAL, +0.2 dB on the chord a peak, 16' rows 2.5 dB low) | VERIFIED (run) |

Key-down state the stage writes: weight `1/48 * velocity gain` (linear: the processor is linear in the weights) at `0x95FE4 + key + {0, 0x62, 0xD0}` for keys 24..108, the three bias words `0xC2D58 / 59 / 5C` (`-0.00958`) while any key is held; key-up writes 0 (a hard step, ASSUMED: the firmware key-up was not captured for model 2), 10,000 calls (0.46 s) of ring-down before the stage idles (ASSUMED, as the Bass), the lattice flag stays 0 (the image value; what sets 1 is not known, so ASSUMED the default). The divider runs only while the stage is active, so an idle divider is frozen. Not modelled: the Farfisa tab selection (key-down writes the same weights for every tab), the vibrato (LFO words `0xC27D6`, `0xC27D7`).

Defaults. `--farfisa-processor on` stays opt-in (D22): there is no Farfisa hardware clip of a program that plays the Farfisa model (the clips of [organ-presence-gap](organ-presence-gap.md) play B3), so `tools/emu_vs_hw.py` has nothing to compare and the on / off delta of the Farfisa band would be by construction. `render_hash` is unchanged.

## 17. Cost of the firmware stages and what was tried (2026-11-15)

Tool: `cargo run --release -p nord-tools --bin organ_fw_bench` (three keys, 4 s of audio at 48 kHz, wall time / audio time; the machine was shared with other builds, so figures vary by about 30 % between runs).

| Stage | Interpreter, 128 bit float | + 64 bit add / multiply fast path (`b3_bass_fw/fast.rs`) |
|---|---|---|
| B3 Bass | 4.3 s CPU per second of audio | 3.3 to 3.6 |
| Farfisa | 2.4 | 1.8 to 2.0 |

`fast.rs` computes `Fx + Fy` and `Fx * Fy` of two normal operands in 64 bit integers (31 guard bits and a sticky bit are enough); VERIFIED equal to the 128 bit routines on 400,000 random operand pairs including cancellation and every exponent distance (`fast::tests::fast_paths_equal_the_general_routines`), and the oracle goldens still hold. **Not real time** (target 0.3 s): the Bass would need about 12 times, the Farfisa 7 times.

Tried and dropped: straight line Rust (one `Machine::exec` call with literal rows per instruction, `#[inline(always)]` so the compiler folds the row dispatch; loops as `for`, forward jumps as labelled blocks, split into functions at positions no jump crosses). It was bit exact against the interpreter and the goldens (Bass 48 calls, Farfisa both tables, whole machine state equal), but `rustc` in release mode did not finish the crate in 10 minutes of CPU and 1 GB (even with 24 line functions), so it was removed. The next candidates are a code generator that emits the arithmetic itself (locals, no `Pending` collection) or a 64 bit float path (a 40 bit value is exact in `f64`; rounding to 32 significant bits after each operation differs from the hardware in about 1 of 2^21 products) with an error bound.

Not done: the drawbar / percussion dependence of the Bass key tables and the key-up `G` resync (needs new ARM captures of non `888888888` drawbars and percussion; nothing captured in this pass), the Farfisa generator `0x122A71` on the machine (`float` clip, `max`, `fix`, register loop count), `tools/emu_vs_hw.py` (no B3 Bass or Farfisa hardware clip exists; both stages stay opt-in, D22).

## 18. Compiled in place executor: 2.9x (Bass) and 3.1x (Farfisa) cheaper, bit identical, still not real time (2026-11-21)

Checked 2026-11-21. Sources: `organ/b3_bass_fw/compile.rs`, `fastexec.rs`, `fast.rs` (this change), `bench_organ_fw.rs`, [sharc-aot-hot-block](sharc-aot-hot-block.md), [sharc-jit-ceiling-study](sharc-jit-ceiling-study.md). Question: can the Bass (D71) and Farfisa (D74) stages run inside the 0.8 ms block budget (TD-15)?

Tools: `cargo run --release -p nord-tools --bin organ_fw_bench` (three keys, 4 s, one stage alone, wall time over audio time, prints an FNV hash of every output sample) and `ns4 bench --scenario organ-bass` / `organ-farfisa` (the `typical` workload, Scene I, 10 keys, default FX, with Organ A on the stage). Same host, same session, shared with other builds (about 10 percent noise), release build, `main` at `96253e23` against this branch.

| Stage (3 keys held) | Before | After | Machine alone, per call (24,000 calls per second of audio) |
|---|---|---|---|
| B3 Bass | 1.82 to 1.91 s CPU per second of audio | 0.63 to 0.65 | 77 us before (whole stage), 27 us after (0.65 s) |
| Farfisa | 1.34 to 1.36 | 0.43 to 0.48 | 17.3 us after zero branching, 11.2 us final (0.27 s); the rest is the `FarfisaWheels` ring (0.16 s) |

Block cost (`ns4 bench`, 2000 blocks of 128 frames, 4 outputs, mean ms per block, 3 runs each, deadline 2.667 ms, PRD budget 0.8 ms):

| Scenario | Before | After |
|---|---|---|
| `typical` (no stage, reference) | 0.81 to 0.84 | 0.81 |
| `organ-bass` | 6.3 to 6.8 | 3.1 to 4.1 |
| `organ-farfisa` | 4.0 to 4.4 | 1.6 to 2.0 |

Output hashes equal before and after (`organ_fw_bench`: Bass `0xd1d697626e610cb9`, Farfisa `0x022b186501a4479b`; `ns4 bench`: `typical` `0x37d60a94ae3b99bd`, `organ-bass` `0x734b8a69c5587b8e`, `organ-farfisa` `0x6cc62194478311d7` on both builds).

What was done, in the order it paid:

1. **Zero operands in the 64 bit path** (`fast.rs`). A zero (exponent 0) fell into the 128 bit general routine. 40 percent of the adds and 45 percent of the multiplies of the Bass chord have a zero operand (1,184 adds and 1,204 multiplies per call, counted with a temporary counter). A zero is now one early branch (zero times finite is a signed zero, zero plus x is x). 1.60 to 1.34 (Bass), 0.96 to 0.68 (Farfisa, whose weight sums are mostly zero: only held keys are non zero).
2. **A flat compiled program with in place execution** (`compile.rs`, `fastexec.rs`). The reference executor (`exec.rs`) collects the writes of an instruction in a `Pending` buffer to give every item the old state; that buffer, the 256 byte register copy of a conditional instruction and the two level dispatch cost more than the arithmetic. At setup, each instruction's items get a read set and write set (bit masks over data registers, `I M L B`, memory, the accumulators, flags) and are ordered so that plain in place execution reads each value before it is overwritten; an instruction with a cycle, a double register write or a condition keeps the reference path (5 of the 236 generator instructions, 0 of the processor's). All items sit in one flat list (one dispatch per item, hardware loops and jumps are control ops). VERIFIED: the compiled machine and the row machine hold identical registers, index registers, flags, accumulators and the whole DM after every one of 600 chord calls (`fastexec::tests::compiled_chain_equals_the_reference_machine_state_for_state`) and over both Farfisa lattice tables (`farfisa_fw::tests::compiled_processor_equals_the_reference_executor_state_for_state`); the oracle goldens still pass. The reference executor stays as the test oracle.
3. **No data dependent branch in the float rounding** (`fast.rs`). The cost was data dependent branches (rounding, operand order, add or subtract), not the arithmetic: a micro benchmark of the routines on random operands gave 14 ns per multiply plus add pair with branches, 10 to 11 ns without. Rounding is one add and one shift (`(m + half - 1 + lsb) >> s`, sticky or-ed into bit 0, valid for `s >= 2`; the deep cancellation case with `s < 2` is a cold function), the operand swap and the add / subtract choice are arithmetic selects, a product needs no leading zero count (its top bit is 62 or 63), `float` and `PASS` have 64 bit paths (`float_by`, `pass`). 50 to 36 us per Bass call. VERIFIED equal to the 128 bit general routines on 400,000 random pairs, 200,000 zero cases, 200,000 `float` / `PASS` cases (`fast::tests`).
4. **Inlined ALU match and one flat item list** (`fastexec.rs`): the common integer operations and the float ones are inlined with the operation match outside the per element loop (36 to 31.6 us), and the two level dispatch (instruction, then item) became one (31.6 to 29.5 us). The zero branch of step 1 in the Bass processor and generator too: 29.0 to 27 us.
5. Tried and dropped: selecting zero results with masks instead of the branch (Farfisa 17.3 us against 11.2 us with the branch: the zero places are fixed, so the branch predicts); the 128 bit general routines are untouched.

Where the remaining time goes (Bass, 27 us per call, about 2,300 instructions and 4,100 items; Farfisa 11 us, 1,034 instructions): about 5 to 6 ns per float add or multiply (a micro benchmark of 4,096 random operands: 10 to 11 ns per multiply plus add pair, throughput bound, 14 ns before step 3), plus 5 to 7 ns per item for dispatch, register file memory traffic and the loads (removing the whole ALU leaves 13.8 us of the Bass call). Both are the cost of interpreting: an indirect branch per item that the predictor cannot learn over a 4,000 item period, and every register in memory. Neither shrinks without changing the structure.

**Budget.** The cost does not depend on the keys held (a fixed number of SHARC instructions per call) and is paid while a key is down plus the 10,000 call ring-down. At 0.65 s (Bass) and 0.27 s of the stage alone (Farfisa, 0.43 with the wheel ring), a 128 frame block (2.667 ms of audio) costs about 1.7 ms (Bass) and 0.7 to 1.2 ms (Farfisa) on top of the rest of the engine (`typical`, 0.81 ms here): the Farfisa program stays inside the deadline (1.6 to 2.0 ms) but over the 0.8 ms figure, the Bass program is over the deadline (3.1 to 4.1 ms) on this shared host. **Not reached: the 0.8 ms block budget for these programs.** The next steps need a different structure, not tuning:

- Straight line Rust with the registers in locals (s17 failed because the old attempt inlined the full `exec` per row; a generator that emits only the float and integer arithmetic and the addressing would remove the dispatch and the register file traffic, about half of the 27 us, and let the predictor see the zero pattern per site). Cost: a 10,000 line GENERATED source per routine (a size rule exception like `prog_proc.rs`), compile time unknown.
- SIMD across the two PEs (the same instruction on both elements): SSE2 has `pmuludq` for the product but no lane wise variable shift or leading zero count, so it needs AVX2 or NEON paths and runtime detection.
- A closed form of the Farfisa processor (weights times ring, three sums, biquads, lattice): the structure is linear in the weights, so the zero weights can be dropped exactly only if the sum order is kept; this is where the 45 percent zeros could disappear entirely.
- A second core (the stage output is needed one DSP call ahead): needs a lock free pipeline, outside the real-time rules of this task.

New in this change: `ns4 bench --scenario organ-bass` and `organ-farfisa` (`bench_organ_fw.rs`), so the block cost of these programs is measured with the same tool as TD-14.

## 19. Planner for cycles and conditions, straight line generated routines: Bass 0.65 to about 0.4, Farfisa 0.45 to about 0.25 (2026-11-21)

Checked 2026-11-21. Sources: `organ/b3_bass_fw/{plan,compile,fastexec,native_gen}.rs`, generated `native_bass_gen.rs`, `native_bass_proc.rs`, `native_farfisa.rs`, `bench_organ_fw.rs`. Question: how much of the s18 remainder (interpretation overhead) can be removed without touching one output bit?

Profile first (release test timing of 20,000 compiled calls on the oracle chord, shared host): generator 26.3 us, processor 11.7 us per call. Removing the float arithmetic (replace the multiply and add results by an xor, timing only) left generator 12.3 us, processor 5.4 us: the float operations are a third of the generator and half of the processor, the rest is interpretation. The op histogram showed the generator still sent about 90 instruction executions per call through the reference executor (`ctl::REF`): 22-pass loop of multiply and add pairs that read each other's destination (a cycle for the in place order), a store and a load trading through the same register, and two conditional ALU items (the reference path copies the 256 byte register file for every conditional instruction).

What was done, in the order it paid:

1. **Cycle breaking** (`plan.rs`). An item with a deferred write is split into a compute half (reads now, result into one of two temp slots of `Machine::tmp`) and a commit half. ALU, store and load items can be split; the planner tries no split, then every single item, then pairs, and keeps the first order that exists. A swap or a double register write still keeps the reference path. A conditional instruction that is one ALU item runs as `COND_ALU` (per element, `AN & !AZ`, no register file copy). Generator after: 26.3 to 16.1 us per call; the Bass and Farfisa programs have no `REF` left (`compile::tests` bound it).
2. **One dispatch per ALU item** (`ALUK + operation` kinds, const generic operation): small (processor 11.7 to 9.5 us).
3. **Straight line generated routines** (`native_gen.rs` writes `native_*.rs`). Each compiled op becomes `m.run_item(&[literal row])`, the loops are Rust `for`, the forward jumps of the Farfisa routine are labeled blocks. With a literal row and `#[inline(always)]` the kind match, the operand decoding and the register bounds checks fold away; there is no interpreter dispatch left. The DM is a boxed fixed size array so a literal address needs no bounds check. The checked in files are 350, 557 and 630 lines (GENERATED, a size rule exception like `prog_proc.rs`; compile time of `nord-core` release grows by about 25 s). `native_gen::tests` fail when a file is stale (`NS4_REGEN=1 cargo test -p nord-core native_gen` rewrites them).

Measured (release, `organ_fw_bench`, three keys, 4 s; the host was shared with six other sessions, so ranges): Bass 0.78 to 0.87 s at the start of this session (0.63 to 0.65 s on a quiet host, s18) to 0.38 to 0.52 s; Farfisa 0.35 to 0.50 (0.43 to 0.48 quiet) to 0.22 to 0.33 s including the 0.16 s `FarfisaWheels` ring, so the Farfisa firmware stage itself is about 0.06 to 0.15 s.

Output hashes equal before and after: `organ_fw_bench` Bass `0xd1d697626e610cb9`, Farfisa `0x022b186501a4479b`; `ns4 bench` (same base, `typical` `0xc3b747edc48dc9f1`, `organ-bass` `0xd9cddec340656bbf`, `organ-farfisa` `0xf85573a9387d95f7`). Mean ms per block (128 frames, 2 runs each, this host): `typical` 1.28 to 1.36 before, 1.05 to 1.15 after (noise: the engine did not change), `organ-bass` 4.2 to 4.3 to 2.5 to 2.8, `organ-farfisa` 2.4 to 3.1 to 1.45 to 1.67.

VERIFIED bit identical: `native_gen::tests::the_native_bass_routines_equal_the_interpreter_state_for_state` (600 chord calls, registers, index registers, flags, accumulators and the whole DM compared after every call, output equal) and `farfisa_fw::tests::the_straight_line_routine_equals_the_interpreter_state_for_state` (300 calls, both lattice tables, changing ring), plus the existing goldens against the oracle.

**Budget.** Not reached for Bass. The Bass stage costs about 1.0 to 1.4 ms per 128 frame block (0.38 to 0.52 s of CPU per second of audio times 2.667 ms) against the 0.8 ms of the whole PRD budget; the Farfisa stage with its ring about 0.6 to 0.9 ms. Remaining gap for Bass: 1.3 to 1.7x on the stage alone (the engine around it is 0.8 to 1.1 ms on this host). Next ideas, in order of expected gain: (1) unroll the generator loops (22 and 10 passes) so the index registers and addresses become literals and the per-key zero pattern is visible per site; (2) the 40 bit float add and multiply are now about half of the remaining time: SIMD across the two PEs (AVX2 / NEON with runtime detection) or a leading zero count free round; (3) skip the work of keys that are not held exactly (the generator has 24+ voices of which three sound): needs a proof that a zero weight leaves every state word bit identical, as for the Farfisa closed form; (4) a second core behind a lock free queue.

## 20. Unrolling, skipping unheld voices and SIMD on the straight line routines: none pays, bit identical, budget still missed (2026-11-21)

Checked 2026-11-21. Sources: `organ/b3_bass_fw/{native_gen,native_bass_gen,native_bass_proc,native_farfisa,probe}.rs`, `organ_fw_bench`, `ns4 bench`. Question: do the three next ideas of s19 (unroll the 22 and 10 pass generator loops, skip the voices that are not held, SIMD over the two PEs) bring the Bass stage under the 0.8 ms block budget? Answer: no; the code is unchanged apart from a test only probe, so every hash is unchanged.

Baseline of this session (release, shared host, `main` at `25bfa8ee`): `organ_fw_bench` Bass 0.362 s, Farfisa 0.195 s of CPU per second of audio (hashes `0xd1d697626e610cb9`, `0x022b186501a4479b`, equal to s19). A release test timing of 20,000 calls on the oracle chord: generator 9.5 to 9.7 us, generator plus processor 16.9 us per call (0.41 s per second of audio at 24,000 calls). `ns4 bench` (2000 blocks of 128 frames, host loaded, so only the hashes are comparable): `typical` 2.64 ms `0xc3b747edc48dc9f1`, `organ-bass` 4.06 ms `0xd9cddec340656bbf`, `organ-farfisa` 2.24 ms `0xf85573a9387d95f7`; the three hashes equal s19.

| Idea | Experiment | Result |
|---|---|---|
| (1) Unroll the loops | `native_gen.rs` emitted every loop of 10 or more passes (generator 22 and 10, processor 13 and 16, Farfisa 20, 23, 23, 22) as a local `macro_rules!` holding the body and one expansion per pass (the checked in files stay short), regenerated, state for state test green | **Slower, 2.0 to 2.3x**: generator 9.5 to 15.0 us, generator plus processor 16.9 to 36 us per call (three runs 35.9, 36.5, 36.9). Reverted |
| (3) Skip unheld voices | `probe.rs` runs the compiled generator item by item and compares registers, accumulators, flags and the whole DM before and after every pass of the 22 and 10 pass loops, over 40 calls with the oracle chord held and with no key | **No pass is an identity** in either case. The 10 pass loop writes two DM state words per pass, with keys and with none; the 22 pass loop moves the `d` registers in every pass (no key; with keys it also writes 0.0 to 2.5 DM words per pass). Test `probe::no_pass_of_the_long_generator_loops_is_an_identity_with_keys_or_without` |
| (2) SIMD over the PEs | Census of the float items of the generator (same probe, 40 calls): add and subtract items 251 per call, 83 percent with no zero operand in either lane; multiplies 228 per call, 41 percent with a zero operand in both lanes, 18 percent in one | Not built: see below. Test `probe::float_adds_of_the_generator_rarely_have_a_zero_operand_but_multiplies_often_do` |
| Build profile | `codegen-units = 1`, `lto = "fat"` (separate target dir) | Bass 0.432 s, Farfisa 0.217 s: no gain inside the 30 percent host noise. Not adopted |

Why unrolling cannot help here. The goal was literal addresses and index registers. The loop bodies do not have them: they load index registers from DM pointer words inside the loop (for example the item `[4, 34, 1, 13, 9, ...]` of the 22 pass loop loads `I2` from the table at `I13`), the voice record pointers of s15 that rotate with the press count. The addresses are data, so unrolling only repeats the same code 22 times; the 2x slowdown is the larger instruction footprint (the generated routines grow several times; the instruction cache is the likely cause and was not measured separately). The per key zero pattern is not static either: the same sites have zero operands for some keys and not for others.

Why skipping is not exact. A pass could be replaced by its effect only if the effect were nothing. Every pass changes at least one of the 40 bit registers, the accumulator or flags, or writes DM, with keys and without: the lattice sections and the scanner phase keep running, the state words carry audio history. The "24 voices of which three sound" of s19 is the section count; a section with a zero weight still passes its state along. A skip would need the closed form effect of each pass on every state word, which is a new model, not a skip.

Why SIMD is bounded. Float add and multiply are about 480 items per generator call (251 + 228) at 5 to 6 ns each, i.e. 2.4 to 2.9 us of the 9.5 us generator call (the processor was not counted; s19 measured the float arithmetic as half of the processor). A two lane vector routine can remove at most about a third of that, so at most about 1 us of 17 us per call (about 6 percent), and it loses the per lane zero branch that s18 measured as the win of the Farfisa weight sums (17.3 to 11.2 us). The estimate is not measured; building AVX2 and NEON paths with a leading zero count emulation, a scalar fallback and a cross check test for 6 percent was judged not worth it. What dominates is the 5 to 7 ns per item of register file loads and stores through `&mut Machine` and the DM loads.

**Budget.** Not reached, unchanged from s19: Bass stage alone 0.36 to 0.41 s of CPU per second of audio, i.e. 0.96 to 1.1 ms per 128 frame block against the 0.8 ms of the whole engine (1.2 to 1.4x over on the stage alone, the engine around it adds 0.8 to 1.1 ms); Farfisa stage plus ring 0.195 s (0.52 ms per block, inside the budget alone). Next ideas, in order: (a) registers in locals: the generated function keeps the 32 data registers and the index registers in local variables (a struct passed by value through macros instead of `&mut Machine`) so LLVM keeps them in machine registers across items; the item bodies change shape, not the arithmetic, so the state for state tests carry over; expected gain unknown, the register file traffic is the biggest part left; (b) a second core behind a lock free queue (the stage output is needed one DSP call ahead); (c) a closed form of the Farfisa processor.

## Implications

- The Farfisa generator is fully decoded and ported; the Farfisa audio is not reachable until the ARM weights and the filter table are captured (OQ19). The documented structure (four weighted sums, biquads, 8 lattice sections, four gains) gives the model target.
- The B3 Bass sounds with the ARM data we have; its processor holds the scanner. A bit-exact model of `0x122296` (and `0x121E2B`) is the path to the B3 / Bass default; the B3 wheels (`B3Wheels`) already supply the ring.
- `docs/research/b3-generator.md` follow-up 1 (writer of `DM 0x95471..`) is closed: the engine constructor `0xC002187C`.
- The DSP frame rate is probably about 43,403 Hz (Farfisa and Vox agree); the Pipe bank rate (43,156.4 Hz) is 9.9 cent off that reading (ASSUMED).

## Follow-ups

1. Find the ARM writer of the Farfisa weights `DM 0x95FFC + 0x178 i`, filter tables `0x965FC..` and the lattice flag (the sender `0xC0224468` family with the record table `0xC0282ED0`, [pno-full-pass](pno-full-pass.md) s6), then replay the Farfisa chord and port the processor.
2. Bit-exact Python models of `0x122296` and `0x121E2B` on random inputs (the layout is in section 4.2), then the scanner as a Rust stage on `B3Wheels`. Section 13: the listing of `0x122296` runs bit-exact in `tools/sharc_listing_vm.py` for one call; the generator listing and the multi-call replay are done (s14, 4696 calls bit-exact); next are branches for `0x122B85`; s15 runs the decoded generator and processor in Rust (the Bass stage, scanner phase law read from the listing).
3. Identify what the second store of an organ layer edits (chan 1, 3).
4. Record the real frame rate (a C0 / C4 recording) and re-anchor `DSP_FRAME_RATE`.
