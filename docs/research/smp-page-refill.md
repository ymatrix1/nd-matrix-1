# SMP page refill: descriptor law, flash feed and the FIFO replay bug (OQ41, D191, D199)

Checked: 2026-11-21 (D199 update: s4 rewritten, root cause found). Sources: SMP image listing `0x3818B4..0x381AB4` and `0x381B00..0x381CCA` (`target/oracle/smp.bin`, local), `.npno` files under `.reference/` (local only), `sharc-live --smp --epchain` runs of the four programs of [piano-chain-tap-smp](piano-chain-tap-smp.md), [smp-voice](smp-voice.md), [nsmp4-and-npno-cnsp-codec](nsmp4-and-npno-cnsp-codec.md).

Question: what does the SMP need so that the source ring of a voice holds the `.npno` page data, and does the piano chain tap then carry audio?

## 1. Short answer

| Part | Result | Tag |
|---|---|---|
| Descriptor to flash law | solved, see s2 | VERIFIED (run, 4 files) |
| Flash feed in `sharc-live --smp` | works: `L` loads a `.npno`, `F` places it, the page routine reads header and data and fills the stream rings with the file words | VERIFIED (run: ring head equals file payload, `tools/arm_smp_flash.py check_rings`) |
| Chain tap carries audio | **yes (D199).** The voice never froze: the harness replayed stale FIFO pairs forever and the key start request reset the voice every 10 passes (s4). With the FIFO index and cap fixed, all four programs give a varying tap past the first refill (peaks -7.75, -13.00, -7.36, -19.11 dBFS, 23,872 words each; `arm_smp_flash.tap_stats` kind `audio`) | VERIFIED (run, `data/smp-page-refill.csv` columns `*_fixed`) |
| `piano_no_chain_tap` | unchanged, 155 programs | |

## 2. The law

The ARM writes the stroke descriptor into the voice record: `rec[-11]` directory index of the file, `rec[-10]` page number of the stroke, `rec[-9]` frame count (`+0x0E` of the stroke record). The page routine (`0x1C0C25`, `0x1C0DA8`) computes

`R7 = DM(0xB237B + rec[-11] + (rec[-10] >> g)) + (rec[-10] & (2^g - 1)) * s`, `(g, s) = (6, 0x3FE)` stereo, `(7, 0x1FF)` mono.

Latch `R9 = R7 >> 24` goes to bank 2, `R7 & 0xFFFFFF` addresses bank 1 (page header word) and bank 3 (data). All banks are one 16-bit-word flash (listing, VERIFIED). `rec[-10]` of a stroke is `(file byte of the stroke + 148) / (1022 * channels)`, the file byte including the `0x2C` CNSP base (`tools/arm_smp_flash.py::stroke_page`). The ARM owns the flash file system, so the directory is a harness choice: `dir[i] = 65408 * i` (a group of 64 stereo or 128 mono pages) and `base = 65408 * rec[-11] + 74`.

Check: every descriptor seen in 4 runs (3 stereo files, 1 mono file) matches a stroke by frame count and `rec[-10] == stroke_page` (`law_ok` column of `data/smp-page-refill.csv`), and the ring head equals the file payload (`check_rings`).

## 3. The feed

`crates/nord-sharc/src/core/periph/smpflash.rs` serves banks 1 to 3 (core accesses and DMA chain reads), `runner/live_flash.rs` implements the protocol lines `L PATH` (load), `F REC11` (place and write the directory), `E` (report, `flash_b1`, `flash_b3`, `flash_outside`), `C PATH` (capture the tapped f32 frames), `H 0|1` (ring half). `tools/arm_smp_flash.py` watches the SMP messages, recognises the descriptor, and sends `F`. `arm_dsp_live.py --piano-bind BACKUP` attaches it and writes `smp_flash` into the result.

## 4. Why the tap was silent: the harness replayed the command FIFO (D199)

The earlier reading (s4 of D191) was an aliasing error: the record was sampled at passes that are multiples of 10, and the voice period is 10 passes. Stepping every pass (clavinet, key script replay) shows the counter running `0, 1, 3, 4, 6, 8, 9, 0xB, 0xC, 0xE`, then `0, 0, 0` (step fraction, counter, phase) and again from 1. The voice is reset every 10 passes (20 audio interrupts).

| Step | Evidence (`sharc-live --smp`, new commands `W`, `S`, `G`) | Tag |
|---|---|---|
| The writer of the reset is not the page routine | `W 0x93908 4` shows the writes `0x1C0B0B..0x1C0B14` (zeroing, the kill routine `0x1C0B08`) then `0x1C0BCA`, `0x1C0BD0` (start from the request entry `0x9251A`) | VERIFIED (run) |
| The request entry is filled by the FIFO consumer, not by the ARM | `W 0x9251A 0x18` over passes 700 to 840: nonzero writes by pc `0x1C05EA` (the opcode 0 store of the FIFO loop) at passes 815, 825, 835, ... although no ARM message is sent after pass 814 | VERIFIED (run) |
| The consumer replays the ring | probe `0x1C05D7` (loop of the FIFO batch) is entered in **every** audio interrupt from pass 0; `S` snapshots at `0x1C05CC`: `R1 = 0x10` (cap), `R2 = 0x280` (ring length), `R3 = 0x80000`, `I0` advances 16 pairs (`0x20` words) per interrupt around the 640 word ring | VERIFIED (run) |
| Cause 1: index bias | The ISR computes `n = (DM(0x2400) + R3 - I0) >> 1` with `R3 = 0x80000`. The harness stored the plain write address, so `n` was `0x3FF..` and the cap 16 applied in every interrupt. The ring (640 words = 20 interrupts of 32 words) was walked again and again: period 10 passes. Fix: `DM(0x2400) = write address - 0x80000` (`live_smp::INDEX_BIAS`) | VERIFIED (run: `n = 0` once the batch is consumed, no replay) |
| Cause 2: cap word | The model capped a batch with `DM(0x92180)` (SY1SY2 word, 65 pairs fed in one pass on SMP) while the SMP ISR caps at `DM(0x92419) = 16` (listing `0x1C05C9`). Pairs beyond 16 were lost once the replay no longer hid it. Fix: `live_smp::FIFO_CAP = 0x92419` | VERIFIED (listing, run: 105 resets, all 1,594 pairs delivered) |

Why the voice looked frozen at the time: the replay delivered the start request (`0x92526 = 0x93928`, step `0xCE430A00`) again every 20 interrupts; each time `0x1C0B08` killed the record and `0x1C0B8D` restarted it at counter 0. The page routine itself, the flash feed and the descriptor law were right. The clavinet "DC plateau" was the output of a voice 15 samples into a page, repeated.

After the fix (clavinet record, same script): counter `0x31` (pass 845), `0x89` (900), `0x12A` (1000), `0x26D` (1200), `0x410` (1460); `rec[-10]` goes `0x802` to `0x803` at pass 1460 (page 2051); `flash_outside` is 0 for all four programs (the 129 reads of D191 were stale-replay descriptors).

### Result of the four programs (3,000 key ticks, tap of the SMP post stage ring, `data/smp-page-refill.csv`)

| Program | Layer file | Tap peak (dBFS, L/R max) | Kind | Native peak (D185 table) |
|---|---|---|---|---|
| Astoria Grand | Astoria Grand .npno | -7.75 | audio | -8.94 |
| Classic Wah Clav | Clavinet .npno | -13.00 | audio | -16.97 |
| Felt Upright | Felt Upright .npno | -7.36 | audio | -8.75 |
| Bubbles Mk I | EP2 Ideal Timbre .npno | -19.11 | audio | -5.98 |

The levels are **not** a parity claim: the tap is the pre-chain post stage ring of one layer, the harness plays note 60 at velocity 100 with layers 2 and 3 not separated, and the native column is a different measurement. Pitch is unchecked: the autocorrelation of the clavinet and upright taps gives about 150 Hz, note 60 is 261.6 Hz, so either the tap frame rate, the stroke choice or the voice step is still off (follow-up 2). The tap kind is `audio` by shape only (more than 64 distinct values in the loudest 4,096 word window).

Not checked: the SY1SY2 chip uses the same ISR code (`0x1C0638` loads `DM(0x2400)`, adds `R3`); its model still stores the plain address (`index_bias` 0) and its cap word `0x92180`. Whether its `R3` is also `0x80000` and whether SY runs replay stale pairs is open (follow-up 4); the SY census numbers (D153 onward) were not rerun.

## Reproduce

```
cargo build --release -p nord-sharc --bin sharc-live
python tools/arm_dsp_live.py ... --piano-bind BACKUP --key-ticks 3000   # smp_flash in out.json
python -m pytest tools/tests/test_arm_smp_flash.py
```

`out.json` now carries `smp_flash.tap` (`words`, `peak_dbfs`, `kind`). Debug commands used for s4 (line protocol of `sharc-live`): `W ADDR N` / `W` (write watch with the writing pc), `S` (register snapshots at the first `--probe` pc inside the audio interrupts), `G` (registers), probes counted as `ihits=` in the `P` reply. A local replay of the `SMP0.script` of a run with `M`, `P 1`, `D` per pass reproduces the table of s4 in about 20 s.

## Implications

- OQ41 blocker 3 is closed: the data reaches the ring bit-exactly and the voice consumes it; the tap is non-silent and varying for all four programs.
- The harness bug (FIFO index bias, SMP cap word) is fixed in `live_sy.rs` / `live_smp.rs`.
- No ranking column yet and no change of `piano_no_chain_tap` (levels and pitch unverified, layers 2 and 3 not separated).

## Follow-ups

1. Re-measure the four programs against hardware levels once layers 2 and 3 are separated; then run the 155 piano programs with `--piano-bind` and add the ranking column (drops `piano_no_chain_tap` per program).
2. Pitch: the tap shows about 150 Hz for note 60 on clavinet and upright. Check the tap frame rate (2 frames per interrupt at 48 kHz is ASSUMED), the stroke chosen for velocity 100 and the voice step (`rec[-32..-31]`) against the key table.
3. Separate piano layers 2 and 3.
4. SY1SY2: read `R3` in its audio ISR with `S`, apply the same bias if equal, and check whether the SY census replayed stale pairs.
5. The EP DMA completion interrupt (vector 20, `0x1C0A71`) is still not modelled; with the voice advancing, check whether a long note (more than one page, 2051 reached at pass 1460) starves.
