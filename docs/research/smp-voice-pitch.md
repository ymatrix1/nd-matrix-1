# SMP voice pitch: the 150 Hz was a zero-stuffed tap, and the D199 tap was a runaway decode (OQ41, D205)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`, factory restore EEPROM, `target/oracle/smp.bin` (SMP image), `target/PNO/core1_sram.bin`, factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (`.npno` members and programs, read only). Code: `crates/nord-sharc/src/runner/live_smp.rs`, `live_sy.rs`, `tools/arm_smp_flash.py`, `tools/arm_dsp_live.py`, `tools/smp_voice_replay.py` (new), `crates/nord-tools/src/bin/smp_stroke_dump.rs` (new). Earlier notes: [smp-page-refill](smp-page-refill.md) (D191, D199), [piano-chain-tap-smp](piano-chain-tap-smp.md), [smp-voice](smp-voice.md).
- **Question (OQ41, follow-up 2 of D199):** why does the SMP chain tap show about 150 Hz for note 60 instead of 261.6 Hz, how loud is it against the native piano, and can the 155 `piano_no_chain_tap` programs get a tap column?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Why 150 Hz? | Two separate faults. (a) The tap kept 2 frames per audio interrupt, but the ISR writes one (`+0` of the half it holds); the second slot and the other half are 0.0 in every interrupt (0 of 5,968 nonzero). Read as 48 kHz frames the pitch was half: 288.6 Hz / 2 = 144.3 Hz, the "about 150 Hz" of D199. (b) The D199 tap was not a piano at all: it was a **runaway decode** (wrapped sawtooth, see s2). | VERIFIED (run, `data/smp-voice-pitch.csv`) |
| Cause of the runaway | The ARM writes the start history of a voice (`0xC0500 + 16 v`, 16 words) **after** the start request in its message order. The SMP kill routine (`0x1C0B3E..0x1C0B5C`) zeroes all 16 words when the request executes. `sharc-live` delivered a key's ~150 pairs in one burst, so the request ran after the first history words had been stored and wiped them. The order 3 predictor `y = 3 y1 - 3 y2 + y3 + r` keeps any start error as a quadratic, so the output runs away and wraps at +-0.4. | VERIFIED (run, `W` watch: writes `1c05ea` then zeroing `1c0b3e..1c0b5c`) |
| Fix | `sharc-live --fifo-pace N` feeds at most N queued ARM pairs per audio interrupt (the ARM does not deliver 150 pairs between two interrupts); the SMP chip spec passes `--fifo-pace 1`. With 1 or 2 pairs per pass-pair the live history ring equals the reference decode of the stroke bit for bit; burst delivery or 6 pairs per pass runs away. | VERIFIED (`tools/smp_voice_replay.py`: pace 2 `MATCH`, burst `DIFFERENT`; pace 3 loses the start, pace 4 matches) |
| Pitch with the fix | Tap fundamental 0.00601 to 0.00604 cycles per frame in all 12 programs (0.5 % spread), the step word is the same law in all: `0.80618 * 2^(tuning / 1200)`. The Hz value needs the interrupt rate: 288.6 to 290.0 Hz at 48 kHz (+1.7 semitones), 265.1 to 266.5 Hz at 44.1 kHz (+24 cents) against 261.5 to 262.2 Hz for the native render; exactly 261.6 Hz at 43.4 to 43.5 kHz. **Rate answered by D211: it is the DSP frame rate 43,402.34 Hz** ([smp-interrupt-rate](smp-interrupt-rate.md)) | EMPIRICAL |
| Level against the native piano | Tap RMS over the first 3,000 frames is 5.6 to 13.3 dB below the native render (median -9.9 dB). Not a parity number: the tap is layer A before the chain, the native render has layer B, amp, EQ, compressor and reverb | EMPIRICAL |
| Census of the 155 programs | **Mechanism done, run not done.** `program_rank_chain.py --smp IMAGE` runs the SMP chip for piano programs, `program_rank.py` drops `piano_no_chain_tap` when the tap has audio and adds `smp_peak_dbfs`, `smp_cycles`. A pilot of 12 programs (9 sounds: grand, upright, clavinet, EP, mono and stereo) gives audio in 12 of 12. The full 155-program run (about 3 min of CPU each) was not started | EMPIRICAL (pilot) |

## 2. The D199 tap was a runaway decode

Astoria Grand, note 60 velocity 100, 3,000 key ticks, tap of D199 (no pacing, 2 frames per interrupt):

| Check | Value | Tag |
|---|---|---|
| Waveform | smooth exponential growth from 1e-5 to 0.41 over 1,200 samples, then a **sawtooth that wraps** at +-0.4 (values `-356 -67 223 -309 -14 281 ...` per mille); no harmonic series, the spectrum is broadband | EMPIRICAL |
| Reference decode | `smp_stroke_dump FILE 513 OUT` (stroke 513: root key 60, rate 35,000, stereo, 99 pages, `init_left [0, -41, -57, -9]`) is smooth: first samples `-23 -74 -110 -174 -267 -299` (x 8192), fundamental 0.007538 cycles per decoded sample = 263.8 Hz at 35 kHz | VERIFIED (run) |
| Hand decode | order 3 (`c = [0, 1, -3, 3]`, coef row `0xE3A0C`), 9 bit residuals at bit `18 n`, start history `[0, -41, -57, -9]` gives `-23 -74 -110 -174 ...` = reference; the same history **reversed** `[-9, -57, -41, 0]` gives `-60 -196 -356 -583 ...` = the live runaway | VERIFIED (`tools/tests/test_smp_voice_replay.py`) |
| Live history ring, burst delivery | pass 816: `0xC0500..0xC0507 = 0 0 0 0 0 37 -9 -28` (L even slots `0 0 0 -9`, the first 5 words wiped); the decoded ring then drifts `-153 -407 -719 -1132 ...` | VERIFIED (run) |
| Writers (`W 0xC0500 8`) | `1c05ea` stores `c0500..c0504`, then `1c0b4e, 1c0b50 ... 1c0b5c` write 0 to `c0507 .. c0500` (the kill routine), then `1c05ea` stores `c0505..c0507` | VERIFIED (run) |
| Cap not the cause | with the ISR cap raised to 64 the whole group is stored first and the kill wipes all 16 words; delaying the history one pass after the kill gives the exact decode (`counter 24`: even slots `-117 -142 -177 -148 19 -18 -87 -125` = reference `y(16..23)`) | VERIFIED (run) |
| Pacing | 1 pair per interrupt (2 per pass): live ring equals the reference at counter 381 (pass 1100); 4 per pass: equal at counter 419; 3 per pass: the voice never starts (counter 0); 6 and 16 per pass: runaway | VERIFIED (run, `r14` series; the tool reproduces pace 2 and burst) |

`arm_smp_flash.tap_stats` called the runaway `audio` (more than 64 distinct values), so D199 reported a level and a "pitch". The detector cannot tell a wrapped sawtooth from a piano; `tap_fundamental` now gives the pitch of the tap and a runaway has none that repeats across programs.

## 3. Step and pitch law

The ARM writes the step of the voice (`rec[-31]`, request words `0x9251D` / `0x92529`) as the fraction `step_frac / 2^32`, `step_int = 0`:

| Program | Sound | Tuning table (key 60) | `step` | `0.80618 * 2^(tuning / 1200)` | Tag |
|---|---|---|---|---|---|
| Astoria Grand | Astoria Grand Lrg 6.1 | -17 | 0.79830 | 0.79830 | VERIFIED (run, `smp_stroke_dump` prints the table) |
| Bubbles Mk I | EP2 Ideal Timbre Lrg 6.1 | not printed | 0.80618 | 0.80618 (tuning 0) | EMPIRICAL |
| Others | see CSV | | 0.79830 to 0.80897 | | EMPIRICAL |

The base 0.80618 is `35,000 / 43,415`: the nominal stroke rate over a rate between 43.4 and 43.5 kHz. At a 48 kHz call rate the nominal step would be 0.72917 and at 44.1 kHz 0.79365. The measured tap pitch (cycles per frame x step law) is a sounding note of exactly 261.6 Hz only for a call rate of 43.4 to 43.5 kHz. **Answered (D211, [smp-interrupt-rate](smp-interrupt-rate.md) s2):** the SMP interrupt runs at the DSP frame rate `F = 43,402.34 Hz` (ARM literal `0x47298A58`; the SMP serial port is a slave of the board clock). The 4 digit cycle counts above gave 43.4 to 43.5 kHz; the precise tap gives +0.62 cent at `F`, +28.2 cent at 44.1 kHz and +174.9 cent at 48 kHz. The base `0.80618` is `35,000 / F` less 0.50 cent (origin open).

## 4. Pilot census (12 programs, `data/smp-voice-pitch.csv`)

| Group | Programs | Result |
|---|---|---|
| Grand | Astoria Grand, C and G (same sound), Ultimate Ballad (Royal Grand 3D) | audio, -15.9 to -20.0 dBFS |
| Upright | Felt Upright, Shimmer Upright, Frozen Time Whl | audio, -14.9 to -18.0 dBFS |
| Clavinet | Classic Wah Clav | audio, -20.5 dBFS |
| Electric | Bubbles Mk I, 70s Phase Mk I, Rotor EP, J3 HardPan 80 EP, EP Steps | audio, -17.9 to -24.9 dBFS |

All 12: fundamental 0.00601 to 0.00604 cycles per frame, `kind` audio, no halt, no error. Bit-exactness against the reference decode is checked for Astoria Grand only (`tools/smp_voice_replay.py`); the other 11 are checked by the repeating pitch.

## 5. What changed

| Piece | Change | Test |
|---|---|---|
| `live_smp.rs` | tap one frame per interrupt (`FRAMES_PER_ISR = 1`, EMPIRICAL) | `ring_frames_read_left_right_of_the_written_frame` |
| `live_sy.rs`, `sharc_live.rs` | `SyModel::pace`, `--fifo-pace N` (0 = off) | `pace_limits_the_pairs_per_interrupt_below_the_isr_cap`, parse test |
| `tools/arm_dsp_route.py` | SMP chip flags `--smp --fifo-pace 1` | `test_smp_chip_spec_takes_path_1_and_the_smp_flag` |
| `tools/arm_smp_flash.py`, `arm_dsp_live.py` | `tap_fundamental`, `fundamental_cycles` in `smp_flash.tap` | `test_tap_fundamental_finds_the_pitch...` |
| `tools/smp_voice_replay.py` | replay with pacing, compare the history ring with the reference decode | `tools/tests/test_smp_voice_replay.py` (4) |
| `crates/nord-tools` `smp_stroke_dump` | stroke descriptor and reference PCM of a stroke | 2 unit tests |
| `tools/program_rank_chain.py`, `program_rank.py` | `--smp`, `smp` record, columns `smp_peak_dbfs`, `smp_cycles`, flag `piano_chain_no_audio` replaces `piano_no_chain_tap` when a tap exists | `test_program_rank*.py` |

No `nord-core` change, no default change. The committed ranking CSV keeps its old columns until the census is rerun (the native renders for it take about 70 min and 14 GB).

## Reproduce

```bash
C=target/fw_unpacked; B=".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
cargo build --release -p nord-sharc --bin sharc-live -p nord-tools --bin smp_stroke_dump
uv run --with unicorn --with capstone python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/Astoria Grand.ns4p" \
    --live target/release/sharc-live --poll-advance 64 --chip "PNO:1=pno=target/PNO/core1_sram.bin" \
    --chip "SMP:0=smp=target/oracle/smp.bin" --piano-bind "$B" --key-ticks 3000 --log run --out run/out.json
python tools/smp_voice_replay.py run target/oracle/smp.bin target/release/sharc-live --stroke 513 --pace-per-pass 2   # MATCH
python tools/smp_voice_replay.py run target/oracle/smp.bin target/release/sharc-live --stroke 513 --pace-per-pass 0   # DIFFERENT
target/release/smp_stroke_dump run/smp_flash.npno 513 stroke513.f32       # reference PCM
```

`out.json` `smp_flash.tap` holds `kind`, `peak_dbfs`, `fundamental_cycles`; `run/smp_tap.f32` is the tap (f32 L R, one frame per interrupt). About 3 min of CPU per program.

## Implications

- The SMP chain tap is now a real piano signal (history equals the reference decode, harmonic series, same step law in 12 programs). D199 "audio" was a runaway; its levels (-7.75, -13.00, -7.36, -19.11 dBFS) were not piano levels.
- The tap Hz needs the interrupt rate: it is `F = 43,402.34 Hz` (D211). Level parity is not claimed (-5.6 to -13.3 dB, layer A only, before the chain; D211 per class table).
- The 155 programs can get the column with one command per program; the run is the remaining cost.

## Follow-ups

1. ~~Run `program_rank_chain.py --smp` over the 155 piano programs~~ Done (D214): 152 audio, 3 none; `program_rank_smp.py` merges the columns into `program-ranking.csv` without the 70 min native renders ([program-ranking](program-ranking.md) s3.3a).
2. ~~Interrupt rate of the SMP chip~~ answered by D211: `F = 43,402.34 Hz`.
3. Piano layers 2 and 3: D211 shows that the second **voice** (stroke 502) of the same note-on already runs and decodes bit for bit; the release voice (order 4 page) and the second piano layer (`layer_b`, another core and flash image) are open ([smp-interrupt-rate](smp-interrupt-rate.md) s3).
4. Why the real ARM message order works (start before history): real pacing of the ARM writes against the interrupt, so `--fifo-pace 1` is a model, not a measurement; 3 pairs per pass loses the voice start.
5. ~~SY1SY2 chip~~ D211: `R3 = 0x80000` there too; opt-in `--index-bias` and chip token `paced` added, census not rerun.
6. Tap detector: add a "runaway" kind (a sawtooth that wraps) to `tap_stats`.
