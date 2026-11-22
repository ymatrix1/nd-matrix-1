# Synth resonance law per filter type: the quadratic is the LP12 table, not the LP24 one (D214)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** ORG core 2 image of OS v1.66 (`python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR`), `sharc-org` (`cargo build --release -p nord-sharc`), the listing notes of [synth-stage-equations](synth-stage-equations.md) s3 and [synth-filter-links](synth-filter-links.md) s2. Numbers only from the OpenNord project's published closed forms (independent cross-check, no code copied, as in [synth-env-rates-settled](synth-env-rates-settled.md) s5). Derived data: [synth-res-settled.csv](data/synth-res-settled.csv), [synth-res-sweep.csv](data/synth-res-sweep.csv). Tool: `tools/synth_res_oracle.py`.
- **Question:** D212 found that the resonance damping table `B` (`0xB1015`) equals the OpenNord quadratic `k = 2 (1 - v / 128)^2` only below index 38 and left the disagreement open. Which is right for the firmware (table lookup range, clamp, per filter type), and does the native engine need a fix?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Who is right | **Both.** The quadratic is table `D` (`0xB0F94`, the LP12 stage `0x1253BD`) over all 128 entries (max relative error 2.6e-6; `D[128]` repeats `D[127]`). Table `B` (LP24 `0x12566E`, BP `0x125913`, HP `0x125A50`) follows the quadratic only for indexes 0..37 and is `sqrt(0.5) (128 - i) / 64` from 38 (error 2e-5). D212 compared the quadratic with the table of the four-pole stages, not the LP12 one | VERIFIED (tables) |
| What the stage uses | The damping the oracle stages used, `P = 2 R k / g` read from the prologue words every pass, equals the interpolated table at the position word `W28`: `D` for LP12, `B` for LP24, BP, HP. Max relative error 1.0e-7 over 1499 passes per run, positions 0.035 .. 127.965 (both sweep directions, so every index and fraction), all four stages | VERIFIED (oracle) |
| Input gain | `cin` equals `A` (LP12, LP24, HP) or `C` (BP) at the same position, 1.2e-7 | VERIFIED (oracle) |
| Knob to position | Settled position `128 knob / 127` clipped at `127.999` for all 128 knobs (error under 0.036, the residual of the 1500 pass smoother). The knob is not remapped per type | VERIFIED (oracle, 128 of 128) |
| Range and clamp | Index `0..127` plus the fraction toward `idx + 1`; the tables have 129 entries and `B[128] = B[127]` (0.0110), `D[128] = D[127]` (0.000122), so the clip at `127.999` is flat at the top. Position 0 gives `P = 2` | VERIFIED (tables, sweep) |
| Ladder24 | No damping table. The stage coefficients `(-18, -16, -14, -12)` equal `tools/synth_filter_ladder.py::coefficients(R, W28)` word for word on 1499 of 1499 passes per direction; the feedback `k_f = row_4 (W28 / 2^31) / 4` is linear in the position | VERIFIED (oracle) |
| Native tables | Equal to the firmware: the `B` and `D` words equal the research CSVs (existing tests), and the native `Lp4` / `Lp2` damping at the oracle position equals the oracle `P` for all 128 knobs (new test `native_damping_equals_the_oracle_for_all_128_knobs`). **No table fix is needed** | VERIFIED |
| Native position map | The default `TablePos::from_resonance` uses `127 knob / 127`, the firmware `128 knob / 127`. The opt-in `--synth-fw-resonance` (`fw_filter_law::resonance_position`) is the firmware map and equals the oracle for 128 of 128 knobs. At the top the default `B` damping is up to 2 times too high (knob 126: `B[126] = 0.0221` against 0.0115) | VERIFIED (arithmetic on the tables) |
| Default flip | Proposal only, section 5 | proposal |

## 2. Method

1. The pass rewrites the channel 0 frame (`I3 = 0xC2942`) every pass and the stage prologue stores `R` (`I3-30`), `W28` (`I3-28`), `cin` (`I3-26`), `g = 2 R N` (`I3-22`), `k = P N` (`I3-20`) with `N = 1 / sqrt(1 + R P + R^2)`, so `P = 2 R k / g` ([synth-stage-equations](synth-stage-equations.md) s3.1). The stage under test is poked into the slot word `DM(0xC0B8C)`; a sounding voice (note 60) and cutoff knob 80 are set as in `tools/synth_filter_links.py res`.
2. **Sweep.** The smoother (`w10 <- 0.995 w10 + 0.005 w`) moves `W28` from 64 to `64 + 128 w` over the run, so resonance knob 127 sweeps 64.6 .. 127.97 and knob 0 sweeps 0.04 .. 63.4. Every pass is one (position, `P`, `cin`) sample, compared with the table interpolated at the position read in the same pass.
3. **Settled.** Each knob 0..127 is run 1500 passes (LP24 and LP12) and the last pass is stored.
4. **Ladder24.** `R` and `W28` of every pass go to the bit-exact Python model; the four coefficient words are compared for equality.

```
cargo build --release -p nord-sharc
uv run python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram DIR
NS4_SHARC_ORG=target/release/sharc-org NS4_ORG_IMAGE=DIR/core2_sram.bin \
    uv run python tools/synth_res_oracle.py sweep --csv docs/research/data/synth-res-sweep.csv
NS4_SHARC_ORG=... NS4_ORG_IMAGE=... \
    uv run python tools/synth_res_oracle.py settled --csv docs/research/data/synth-res-settled.csv
uv run python tools/synth_scaling_crosscheck.py
```

## 3. Results

Sweep (`synth-res-sweep.csv`; the position range is the min and max read in the run):

| Stage | Knob | Position range | `P` max rel err | `cin` max rel err |
|---|---|---|---|---|
| LP12 `0x1253BD` | 0 / 127 | 0.035..63.362 / 64.638..127.965 | 1.0e-7 / 9.2e-8 | 3.8e-8 / 1.2e-7 |
| LP24 `0x12566E` | 0 / 127 | same | 1.0e-7 / 1.0e-7 | 3.8e-8 / 1.2e-7 |
| BP `0x125913` | 0 / 127 | same | 1.0e-7 / 1.0e-7 | 1.2e-7 / 1.2e-7 |
| HP `0x125A50` | 0 / 127 | same | 1.0e-7 / 1.0e-7 | 3.8e-8 / 1.2e-7 |
| Ladder24 `0x12549D` | 0 / 127 | same | coefficient word mismatches 0 of 1499 / 0 of 1499 | |

Settled damping (`synth-res-settled.csv`; positions from the oracle, quadratic `2 (1 - v / 128)^2` at the same position):

| Knob | Position | `P` LP12 (`D`) | `P` LP24 / BP / HP (`B`) | Quadratic | `cin` LP24 (`A`) |
|---|---|---|---|---|---|
| 0 | 0.035 | 1.99891 | 1.99891 | 1.99890 | 1.000 |
| 32 | 32.269 | 1.11872 | 1.11872 | 1.11870 | 1.000 |
| 37 | 37.306 | 1.00410 | 1.00582 | 1.00408 | 1.000 |
| 38 | 38.313 | 0.98192 | 0.99091 | 0.98190 | 1.000 |
| 48 | 48.387 | 0.77375 | 0.87961 | 0.77372 | 1.000 |
| 64 | 64.504 | 0.49219 | 0.70154 | 0.49216 | 0.982 |
| 96 | 96.738 | 0.11933 | 0.34540 | 0.11930 | 0.484 |
| 112 | 112.855 | 0.02801 | 0.16733 | 0.02800 | 0.234 |
| 120 | 120.914 | 0.006140 | 0.07829 | 0.006130 | 0.110 |
| 127 | 127.965 | 0.000122 | 0.011048 | 0 at index 128 | 0.0155 |

Reading: table `D` (LP12) is the OpenNord quadratic at every index, i.e. their `k` is the damping of one resonant section (`Q = 1 / P` up to 8192 at the top). Table `B` (the two-section stages) is gentler: same start (`P = 2`, `Q = 0.5`), same shape to index 37, then linear, ending at `Q = 90.5`. The four-pole stages therefore peak lower than a quadratic would, and their input gain `A` / `C` (also linear, `0.99 (128 - i) / 64`) keeps the peak bounded.

## 4. Implications

- The native `B` and `D` tables and the per-kind selection (`Lp2` uses `D`, `Lp4` / `Bp4` / `Hp4` use `B`, `Bp4` takes `C`) are right. D212's open point is closed: our table was never at odds with the closed form, the comparison used the wrong table.
- The only native difference is the default position map (`127 knob / 127` against `128 knob / 127`), already available as `--synth-fw-resonance`. No new flag; the new test pins the firmware map and the table lookup against the oracle for all 128 knobs.
- Not modelled (unchanged, effect not measured): the position smoother's time constant (`0.995` per pass, about 200 passes) on knob moves. The native resonance steps instantly.

## 5. Follow-ups

- Flip proposal (owner decision, nothing flipped): make `--synth-fw-resonance` the default for the strip chain. For the LP24 / BP / HP damping `B` it moves `P` by about 0.8 % at knob 64, 3 % at knob 100, 10 % at knob 120 and a factor 2 at knob 126 (the position offset is `knob / 127` index); a listening check on resonance sweeps with LP24 programs should precede it, and the `render_hash` baselines need re-recording.
- A hardware check that would settle Q on the real keyboard: peak height against knob for LP12 and LP24 on a captured sweep.
- Port the smoother only if a clip shows a resonance move click or lag.
