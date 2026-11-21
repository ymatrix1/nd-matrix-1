# Arpeggiator gate flag `[pattern object + 0xC]`: who writes it (OQ32)

- **Checked:** 2026-11-19
- **Sources (local, never committed):** ARM image `target/fw_unpacked/bank0_CODE.bin` + `bank0_SRAM.bin` through the Unicorn harness; the 32 factory programs with an arpeggiator layer (`docs/research/data/arp-morph-flip-sweep.csv`, `arp_layers` non-empty) from `.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b`. Committed numbers: `docs/research/data/arp-flag-census.json` (32 programs), `arp-flag-census-variants.json` (patched variants of A:16).
- **Question:** D130 / D166 kept the 2/3 arpeggiator gate opt-in because the 37 static candidate stores to `[obj + 0xC]` were not traced and the 30 captures of A:16 replay one boot state. Does anything in the firmware ever change the flag, and is the gate 2/3 in every factory arpeggiator program?

Reproduce (needs `unicorn`, `capstone`, the unpacked firmware; about 10 min with 8 jobs):

```bash
C=target/fw_unpacked; P=factory_programs/Program
# one run per program: write hook over the six pattern objects, code hooks on the 37 sites and
# the constructor, flags after boot / load / idle / keys, gate per layer (keys 40 43 47 60 64 67 held 1 s, release at 0.7 s)
awk -F, 'NR>1&&$2!=""{print $1}' docs/research/data/arp-morph-flip-sweep.csv | while read -r n; do
  uv run --with unicorn --with capstone python tools/arm_arp_flag_trace.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --release-at 0.7 --seconds 1.0 --out "target/arpflag/${n//[ \/]/_}.json" "$P/$n.ns4p"; done
# native engine, gate off (--no-arp-gate) and on, same keys, tempo 100, one file per program in target/nat_off, target/nat_on
cargo run --release -p nord-tools --bin arp_trace -- "$P/Bank A/Horizon Whl.ns4p" --keys 40,43,47,60,64,67 --seconds 1.0 --release-at 0.7 --tempo 100 [--no-arp-gate]
python tools/arp_flag_census.py target/arpflag --out docs/research/data/arp-flag-census.json --native-off target/nat_off --native-on target/nat_on
# variants of A:16: --patch A.mode=2 --patch C.mode=2 | A.mst_clk=0 | A.pattern_on=1 | A.rate=20 | A.kb_sync=1 | --force-flag 0
uv run pytest tools/tests/test_arm_arp_flag_trace.py tools/tests/test_arp_flag_census.py
```

## 1. Answer

**The flag is written by the pattern constructor only, and the constructor runs only in boot.** In 41 runs (32 programs, 9 variants) the flag of all six pattern objects stays 1 through boot, program recall, idle, key-on, held arpeggio and release; the write hook saw **0 stores** to `[obj + 0xC]` after boot; the constructor `0xC00FD7B8` ran 6 times in boot (six objects) and **0 times** afterwards. In that state the 53 ARM-played arp layers (48 with a measured fraction, 5 are single notes) of 30 of the 32 arpeggiator programs have a gate of **2/3 of the grid step**; Shoreline Whl and Memory Glitch produced no ARM arp note in the 1 s window (silent in the harness; their flags are 1 like the others).

| Check | Result | Source |
|---|---|---|
| Write hook over the six `0xE0` byte objects (`PATTERN_TABLE` `0xC054CBD0`, 1-based) | stores only at `+0x14` (previous phase) and `+0xBC` (player `0xC023B3EC`, vtable method `0xC023C984`); **0 at `+0xC`** | `arm_arp_flag_trace.py`, `offset_pcs` |
| Flag after boot / load / idle / end, six objects, 32 programs | 1 everywhere | `arp-flag-census.json` `summary.flags_not_one` = 0 |
| Constructor calls | 6 in boot, 0 after | `constructor_calls` |
| 37 candidate stores of D130 (`arm-tick-flagscan.json`), code hooks | 29 reached, 21,908 executions, **0 with a pattern object as base register** | `site_hits_on_pattern_object` = 0 |
| The 8 unreached sites, read statically | `0xC011E350`, `0xC011E3B4`, `0xC011F208`, `0xC011F23C` store the constant 1 as a byte, `0xC011F090`, `0xC011FEBC` the constant 2: non-zero keeps the flag-1 branch (`cmp r2, #0` at `0xC011A544`, `0xC011A658`) even if one hit a pattern object; `0xC023B25C` stores 0 next to `+8` and `+0x10` of a list record (zeroing `+8` / `+0x10` of a pattern object would break its grid, so it is not one); `0xC023B0AC` writes a record `{+4, +8, +0xC, +0x10}` | disassembly (capstone), not data-flow proven |
| Other code that holds a pattern object | the literal references to `0xC054CBD0` (`0xC00FC99C` constructor, `0xC0119940`, `0xC011A520` Glue update, `0xC011C090` / `0xC011C510`, `0xC011DF68`, `0xC01232F0`) only read `+8`, `+0xC`, `+0x10` (Glue update) or write the pattern words `+0x18 +0x1C +0x20`, length `+0x24`, enable `+0x28` and the listener list `+0xC0 +0xC4` | disassembly of the neighbourhood of each use |

What the flag does (Glue update `0xC011A51C`, disassembly): free-run branch: flag 0 selects the straight grid `0xC02BBF98`, non-zero `0xC02BC1A0` (second point `0x55555 / 0x80000` = 2/3); Master Clock branch: the grid pointer `[obj + 0x10]` comes from the table entry `+0x410` (flag 0) or `+0x434` (non-zero). The harness shows `0xC02BC1A0` in free run (`mst_clk = 0` variant) and the Master Clock grids `0xC02BCF20` (rate raw 90 / 96) and `0xC02BD740` (raw 20) on layers A and C. A counterfactual (flag forced to 0 after boot) did **not** change the grid or the gate: the Glue update re-reads the flag only on a rate / mode edit and the run sent none, so it says nothing about flag 0; the meaning of flag 0 rests on the disassembly above.

## 2. Per program (30 of 32 programs played)

`arp-flag-census.json` has one row per program and layer: ARM notes, step, gate (ms and fraction of the played step) and the native engine with the gate off and on. Held keys 40 43 47 60 64 67 at 100 BPM; the tempo is fed to the Master Clock position (the step time of Master Clock layers is an input of the harness, `arp-fw-vs-native.md` s3; free-run layers use the firmware's own rate table).

| Finding | Evidence |
|---|---|
| Every ARM layer that played has `gate_ms` = 2/3 of its grid step. Fraction of the played step: 0.665 to 0.669 for plain steps (the spread is the 24 kHz tick), 0.444 / 0.333 / 0.222 on 9 layers whose notes sit 1.5, 2 or 3 grid steps apart (pattern rests: `step_ms / (1.5 gate_ms)` = 1.5, 2.0, 3.0 exactly) | `gate_fractions`, rows |
| Arp, Poly and Gate mode layers are all 2/3 (Gate mode: Dual Gates, Time Warp Saw, Calling Whl C; Poly: Momentum, Gradient, Unity, Calling, 24 Frames, Helium, 5th Poly Arp, Moving Chords, Synth Engine, Unclocked Pad, Mirror Image Pad, FM Harpies C) | rows; the `poly` and `gatemode` variants of A:16 give 100 ms of 150 ms |
| Free run (`mst_clk = 0`), Pattern on, rate 20, KB sync, Poly, Gate: gate 2/3 of the step | `arp-flag-census-variants.json` |
| Native engine, gate on, against the ARM on the 16 layers where both report the same step (within 1 ms): gate within 1 ms on **13** (gate off: 1 of 16). The 3 that differ: Momentum Whl B, FM Harpies C, Ostinato Whl A (below) | `native_on` / `native_off` of the rows |

Not covered by the gate switch (separate differences the census shows, not changed here): the native `ArpMode::Gate` holds the chord (`tick_gate`, 4/8 duty) while the ARM retriggers it every step with a 2/3 gate (Dual Gates, Time Warp Saw); on Momentum Whl B, FM Harpies C and Ostinato Whl A the native step is 300 ms with gate 150 where the ARM has 300 ms with gate 200; on FM Harpies A, Gravity Whl C and ScissorSplit Whl B the step itself differs. These are rate / pattern differences, not the flag.

`python tools/arp_flag_census.py target/arpflag --out ...` summary of the 32 program runs (`arp-flag-census.json`; the 9 variants of `arp-flag-census-variants.json` add 0 flag writes, 0 flags not 1, 0 constructor calls after boot, 4,664 site hits, 0 on a pattern object; 32 + 9 = 41 runs):

```json
{
 "runs": 32,
 "natural_runs": 32,
 "flag_writes": 0,
 "flags_not_one": 0,
 "constructor_calls_after_boot": 0,
 "site_hits": 21908,
 "site_hits_on_pattern_object": 0,
 "gate_fractions": {
  "0.222": 2,
  "0.333": 5,
  "0.444": 2,
  "0.665": 4,
  "0.666": 1,
  "0.667": 31,
  "0.668": 2,
  "0.669": 1
 }
}
```

## 3. Before / after (384 factory programs)

`program_sweep <backup> --notes 60,64,67 --no-arp-gate` (before) against the same command without the flag (after), compared with `tools/sweep_flip_compare.py` (`docs/research/data/arp-gate-flip-sweep.csv`, derived numbers only): **25 of the 32 arpeggiator programs change** the default render, **0 of the other 352**. The 7 that do not change are Gate-mode layers (Dual Gates, Time Warp Saw, Memory Glitch: the native Gate duty is not the flag) and programs whose arp layers do not sound for these keys (Shoreline Whl, Sidechain Whl II, Night Gaunts Whl, ScissorSplit Whl). `render_hash` (`cargo run --release -p nord-tools --bin render_hash`) is unchanged (`eb13063d...ca28` = `WHOLE` of `tests/golden_render.rs`): the golden segments contain no arpeggiator (`render_segments.rs` has no arp), so it proves only that nothing else moved.

## 4. What is still not proven

- **No hardware recording** of an arp note against its step. The result is the firmware's own state from execution; a path that only a real panel or the unmodelled Master Clock SPI device reaches would not show in the harness. The static reading of the 8 unreached stores and of the table users argues that none sets the flag to 0, but it is not a data-flow proof.
- **Hold, scene change and MIDI clock start** were not driven (Hold is a global bit that is not decoded; scene recall needs the panel path).
- The step time of Master Clock layers is an input (position scale `2^19` per beat, ASSUMED), unchanged from `arp-fw-vs-native.md`.

## Implications

- The 2/3 gate is the firmware's state for every factory arpeggiator program: D168 makes it the default (`--no-arp-gate` and `--legacy-arp` opt out).
- The single-note retrigger was already the default since D166 (part of `arp_firmware`).

## Follow-ups

1. Native `ArpMode::Gate`: retrigger the chord every step with the 2/3 gate (3 programs).
2. Step structure of Poly / pattern layers with rests (6 layers above).
3. Shoreline Whl and Memory Glitch: find keys / zones that sound the arp in the harness.
4. Hardware clip of an arp note against its step; drive Hold and scene recall.
