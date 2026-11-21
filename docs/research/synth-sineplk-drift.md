# `synth_sineplk` baseline drift 4.73 to 7.48: the Mod 2 default flip (D163)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** Part A demo video and factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` in `.reference/`; `tools/emu_vs_hw.py`; data [synth-sineplk-drift.csv](data/synth-sineplk-drift.csv), [mod2-clip-flip.csv](data/mod2-clip-flip.csv).
- **Question:** the `synth_sineplk` gap score was 4.73 at D121 ([synth-filter-links](synth-filter-links.md) s5) and reads 7.48 on `main` 85b2b572. Which change moved it, and is it an intended default change?

## 1. Short answer

**D163: `FirmwareFx::DEFAULT` = amp + Mod 2** (PR 521, merge 6a8ba992, commit b74e4a68 `feat(fx): firmware Mod 2 is the default`). No other default moves the clip. The program `A:26 Sine-Saw Plk Whl` carries a synth-layer Mod 2 with `mod2_enabled = true`, `mod2_type_raw` 5 (CHOR variation), rate 0.7165, amount 0.3701 (`ns4 inspect <ns4p> --json`, `fx_a` / `fx_b` of the synth layer). Before D163 that program ran the engine chorus, after it the firmware CHOR type 5 routine `0x122F77` (`fx/fw_mod2.rs`).

| Check | Result |
|---|---|
| Bisect (release `ns4` per commit, same scorer) | D121 merge `bcf4cbbc` 4.73; `6a8ba992^1` 4.75; `6a8ba992` (PR 521) 7.48; `main` 7.48. PR 519 (`fa5cfd58`, research only) 4.75 on both sides |
| Flag ablation on `main` | `--legacy-mod2` 4.75; every other single `--legacy-*` / `--no-*` flag 7.24 to 7.50 (s3) |
| Split of the drift | 4.73 to 4.75 (+0.02) is the synth velocity flip D142 (already recorded in [synth-velocity](synth-velocity.md)); 4.75 to 7.48 (+2.73) is D163 |

## 2. Reproduce

```
cargo build --release -p nord-core --bin ns4
uv run python tools/emu_vs_hw.py --video "<Part A .webm>" --backup "<factory .ns4b>" --ns4 target/release/ns4 \
    --spec <segments json> --workdir <scratch> --only synth_sineplk
```

`<segments json>` is `docs/research/data/emu_vs_hw_segments.json` with the `synth_sineplk` variant list replaced by `default` plus one variant per flag of s3. Bisect: `git worktree add --detach <dir> 6a8ba992^1` (and `6a8ba992`, `fa5cfd58^1`, `fa5cfd58`, `bcf4cbbc`), `cargo build --release -p nord-core --bin ns4` in each, the same command with `--ns4` set to that binary and only the `default` variant. The scorer tools are those of `main` for every build.

## 3. Ablation (`synth_sineplk`, `main`)

| Variant | Score |
|---|---|
| default | 7.48 |
| `--legacy-mod2` | **4.75** |
| `--legacy-all` | 6.30 (includes `--legacy-mod2`; the other switches it turns off cost the clip about 1.5, not decomposed) |
| `--legacy-synth-velocity` | 7.47 |
| `--no-synth-stages` | 7.24 |
| `--no-synth-knob-laws` | 7.50 |
| `--legacy-arp`, `--legacy-morph`, `--legacy-pump`, `--legacy-routing`, `--legacy-synth-layer-law`, `--legacy-piano-touch`, `--legacy-piano-level`, `--legacy-1khz-tick`, `--no-arp-gate`, `--no-synth-wave-bank` | 7.48 |

The listed D142, D166 (morph, arp), D165 (Touch), D134 (PUMP) and D175 (pan) defaults do not move this clip (at most 0.01 for velocity); `--no-synth-stages` moves it by 0.24.

## 4. What the firmware chorus changes in the clip (hardware minus emulator, `emu_vs_hw.py` rows)

| Quantity | `--legacy-mod2` | default (firmware CHOR 5) |
|---|---|---|
| low 120..400 Hz | +1.6 dB | -0.2 dB |
| mid 400..1500 Hz | -0.1 dB | -6.3 dB |
| presence 1.5..3 kHz | -14.5 dB | -23.9 dB |
| centroid | -68 Hz | -124 Hz |
| RMS | +1.5 dB | +3.0 dB |
| stereo width hw / emu | 0.99 / 0.18 | 0.99 / 0.49 |

The firmware chorus widens the image toward the hardware but moves mid and presence energy away from it, so the gap score, which ranks band errors, rises. Whether the cause is the type 5 port ([fx-mod2-types](fx-mod2-types.md)), the layer routing or the synth layer level into Mod 2 is **not** established here.

## 5. Is it an intended change that helps other clips? No

26 Part A segments, default against `--legacy-mod2` ([mod2-clip-flip.csv](data/mod2-clip-flip.csv)):

| Result | Clips |
|---|---|
| better with the flip | `fx_amp` 4.75 to 3.92, `organ_vox` 2.92 to 2.59 |
| worse with the flip | `synth_sineplk` 4.75 to 7.48, `fx_reverb` 3.67 to 4.23, `organ_pipe` 1.70 to 2.04, `synth_megaline` 2.00 to 2.17, `synth_horizon` 2.19 to 2.34 (and its twin `super_lp24_horizon`), `super_lp24_believer` 1.60 to 1.66 |
| equal | 16 rows |
| mean of 26 | 2.78 (`--legacy-mod2`) to 2.90 (default) |

The flip is **not** a clip improvement: worse on 7 rows, better on 2, mean +0.12. D163 flipped on the oracle null of the Mod 2 routines (level delta 0.00 dB, null -72 dB or better) and the owner approval relayed 2026-11-18, and stated that the flip "moves audio for the 158 Mod 2 programs without a hardware clip to confirm it". The justification is that of D163 (distance to the oracle-verified chain, parity over purity), not a clip score. `synth_sineplk` is the largest clip regression of that flip; D163 did not list it.

## 6. Implications

- No code change: one known, owner-approved cause. No default flipped here (D22).
- To compare with the D121 numbers of [synth-filter-links](synth-filter-links.md) s5 use `--legacy-mod2` for the `synth_sineplk` row (4.75). The other synth rows move by at most 0.17 (`synth_megaline`).
- Filter-variant comparisons on the default build stay valid: the Mod 2 term is the same in every filter variant of a row.

## 7. Follow-ups

1. Find why firmware CHOR type 5 loses about 9 dB of presence and 6 dB of mid against hardware on this synth layer (type 5 biquad port, or the synth layer feed into Mod 2). A hardware recording of CHOR variation on a synth is the evidence that would settle it; the chorus clip in [fx-chorus-hw-clip](fx-chorus-hw-clip.md) is a piano and type 4.
2. Owner decision whether to keep the Mod 2 default given 7 worse Part A rows (D179); `--legacy-mod2` stays the opt-out.
