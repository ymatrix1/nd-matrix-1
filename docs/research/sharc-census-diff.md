# Census diff: every firmware instruction, model against simulator

Checked: 2026-11-17 (rerun and closure of the Open list; first written 2026-11-12; sites in real passes, MR2F, JIT and AOT added the same day, D156). Sources: CCES 2.12.1 ADSP-21489 simulator (headless, `sim_run.py`/`sim_batch.py`), the five DSP images (`tools/ns4_dsp_images.py`), ADI `elfdump` (`tools/adi_dis.py`).
Question: does `crates/nord-sharc` execute every distinct instruction the firmware uses exactly like the simulator?

## Method
`tools/sharc_census.py` tiles the code of the five images (FX, PNO, ORG, SY1SY2, SMP; 6 core sections). `census_diff.py` takes every distinct instruction (parcels), labels it with ADI's own disassembly and drops what ADI does not decode to the same parcels (the tiler cuts data into "instructions"), control flow, timer/stack/interrupt registers and MODE1 writes. 8 287 instructions remain; each runs in 6 random core states (all registers, ASTATx/y with AZ and AN never together, random STKYx/y, MODE1 in 6 variants incl. SIMD, ALUSAT, TRUNC, no RND32; special values mixed in).

`gen_census_probe.py` builds one probe: a loader that sets the whole state from a DM table, one patchable instruction slot (`_slot`), a dumper (107 state words) and a 512-word scratch table. `sim_batch.py` pokes each instruction into the slot of the simulator (`TpsdkServer`, 14 to 16 processes in parallel, 20 minutes for 49 722 cases); `sharc_census_run` (`core/census.rs`) runs the same machine code on a `Core`. `census_diff.py` reports every difference by instruction shape.

## Result
First run: 1 875 of 50 052 cases differed. After the fixes below: **18 of 49 722** (closed 2026-11-17, see below), found and fixed model gaps listed here. All are unit-tested; `census_regress.rs` replays 1 104 cases (278 shapes of the affected families and a 288-case sweep of every condition code over AZ AV AN AC x AF) against the committed simulator output.

| # | Behaviour (VERIFIED by the census) | Model before |
|---|---|---|
| 1 | Loading Bx also loads Ix | B write left I alone |
| 2 | STKYx and STKYy keep bits 0..20 and 27..31 of a written value (stack-status bits 21..26 read-only); ASTATy keeps all 32 bits (ASTATx drops 15..17 and 19..23); STKYy reads the same stack-empty bits as STKYx | masks 0x3E7/0x1E0000, STKYy status 0 |
| 3 | Fixed ALU and fixed COMP clear AI | AI left alone |
| 4 | Every shifter operation clears SF (ASTAT bit 14) | SF kept |
| 5 | Long-word access (`(LW)`: bit 39 of Types 14a, 15a, 15b; bit 30 of 3a, 3b) moves a register pair at `addr & !1`; the named register gets the lower word | word access |
| 6 | SIMD: a store of a non-data register also writes `addr + 1`; an S register transfers like an R register with the roles swapped; immediate loads (Type 17) and immediate stores (Type 16) reach the twin register / next word; STKY immediates also set STKYy | PEx only |
| 7 | SIMD register moves copy the twin (`Sn = Rm` also sets `Rn = Sm`); a conditional move/transfer follows PEx's condition for the R side and PEy's for the S side | one condition |
| 8 | A register transfer and a compute in one instruction both read the old registers (`F12 = F8 * F7, R8 = R0`) | transfer first |
| 9 | Conditions LT, LE, GE, GT depend on AF and ALUSAT: after a float ALU op (AF = 1) LT = AN and not AZ, LE = AN or AZ; AF = 0 with ALUSAT: LT = AN, LE = AN or AZ; AF = 0 otherwise LT = AN xor AV, LE = (AN xor AV) or AZ; GE, GT are the negations | LT = AN and not AZ always |
| 10 | Dual-memory form with a multiplier-plus-ALU compute leaves MN MV MU MI and STKY 6..9 alone; multiplier-only updates them; `MRF = 0`/`MRB = 0` clear MN MV MU MI | always updated |
| 11 | Triple operation (multiply, add, subtract): ALU flags are the OR of sum and difference | flags of the difference |
| 12 | A shift count of `i32::MIN` must not overflow | panic in debug builds |

Provenance in code: each fix carries a `VERIFIED` tag naming the census; items marked ASSUMED stay ASSUMED (AF cleared by a fixed ALU op).

## Open list, closed 2026-11-17
The census was rerun unchanged (same seed, 8,287 instructions, 49,722 cases, 16 simulator processes, 21 min). It reproduced the **18 differences in 6 shapes**; 4 more cases showed `HALT Running` (the simulator did not return, 4 different shapes, not model faults: a rerun of those 4 cases gives sim equal to model). Frequency in firmware code (`census_freq.py`, same tiling as the census, 65,343 instruction occurrences, 8,970 distinct, 6 core sections; addresses are short-word addresses of the first hits):

| Shape (cases) | Occurrences | Where | Audio path | Cause in the model | State |
|---|---|---|---|---|---|
| `IF cc R = LSHIFT R BY 256` (3) | 24 (8 sites x ORG core 1, ORG core 2, PNO core 1) | `0x18253B..0x1825B5` | ORG/PNO shared block, use unknown | immediate count taken as 12 bits | **fixed** |
| `IF NOT MS F0=F0*F4, F0=F8+F12, F15=F8-F12, R14<->S3` (5) | 15 (5 sites x 3 cores) | `0x181E60..0x182018` | same block | conditions MV (5) and MS (6) were `false` | **fixed** |
| `IF cc R<->S` (1) | 9 (3 sites x FX core 1, FX core 2, PNO core 1) | `0x124448`, `0x123AD4`, `0x1247AF` | FX chain and PNO | SIMD swap ran under either element's condition | **fixed** |
| `STKYx = DM(a) (LW)` (6) | 3 (1 site x 3 cores) | `0x1821C4` | same block | pair partner of STKYx | **fixed** |
| `IF cc M12 = DM(I0,M0), R0 = FPACK F0` (1) | 1 | PNO core 1, `0x120E01` | PNO | non-data register in SIMD | **fixed** |
| `MR1F = R3, R1 = DM(..)` (2) | 2 | PNO core 1, `0x1211C4` | PNO | MR2F read-back | simulator artefact, not in the firmware sequence (D156) |

Total 54 of 65,343 occurrences (0.08 %), 15 distinct sites. After the fixes the rerun (model only, simulator output reused) leaves **2 differences** (the MR2F shape) plus the 4 simulator `HALT` cases, which agree with the model when rerun.

Measured by `census_probe.py` (a hand-picked instruction on the simulator and the model, random states):

* Shift immediate (225 cases, 5 opcodes x 15 counts x 3 states): `LSHIFT`, `ASHIFT`, `ROT` and the OR forms take an **8-bit signed** count, bits 11..8 of the immediate are ignored (`BY 256` shifts by 0, `BY 0xFFF` is -1 only through the low byte). The `FEXT`/`FDEP` forms keep their 12 bits.
* Conditions (the triple operation with the swap under all 32 condition codes x 6 states): code 5 (`MV`) is ASTAT bit 7 and code 6 (`MS`) ASTAT bit 6 of the element; their negations are 21 and 22.
* Swap `Rx <-> Sy` in SIMD mode: both halves read the old values, the write to `Rx` follows PEx's condition and the write to `Sy` follows PEy's; there is no second (PEy) exchange.
* Long-word load into ASTATx/y or STKYx/y (`0x76..0x79`): both words load the named register in turn (it ends with the word at `addr + 1`) and the partner register is untouched.
* A conditional load into a non-data register (`M12`) in SIMD mode happens when either element's condition holds.
* Unresolved: code 14/30 (`SF`, `NOT SF`) is true in the simulator in all 12 probe states whatever ASTAT bit 14 holds; the model derives it from the bit FIFO fill level. No firmware code uses it (0 occurrences of `IF SF`, `IF NOT SF`, `IF BM`). `MR1F = R3` with a data move: MR2F reads `0000FFFF` in 4 of 5 negative cases and `FFFFFFFF` when the previous MR2F had bit 15 set (case list in the session notes); ASSUMED to be a simulator register-load artefact, 2 occurrences, and the model keeps the sign-extended read. LW loads of `0x65`, `0x72`, `0x73`, `0x7A` registers differ too (not in firmware).

Regression: `census_extra_cases.txt` / `census_extra_measured.txt` (461 cases, simulator output) replayed by `census_regress.rs::model_matches_the_simulator_on_the_closed_census_diffs`; it fails on 137 cases without the fix. `census_regress.py` now lists only the MR2F shape in `OPEN`, so `census_cases.txt` grew from 1,104 to 1,119 cases.

## Which census sites run in real passes (D156)
`NS4_SITE_HITS=<hex short-word pcs>` (`core/site_hits.rs`, off by default, interpreter only) makes every oracle binary print `site_hits pc=0x.. count=N` for the listed pcs when its core is dropped. The 26 pcs of `census_freq.py` were counted over: `sharc-pass` PNO `core1_sram.bin` (Vox and Pipe installs, 3,000 passes each, 1,500 for the count), `sharc-sys --pno` (316 passes incl. warm-up), `sharc-sys` PNO and ORG `core2_sram.bin` (416 passes, 912 messages of the program-load capture, 8 keys), `sharc-fxlive` FX core 1 (boot log, identity, reverb0, chor4, chor5, 3,000 lead passes) and `sharc-fx` on the FX core 2 ENS routine (96 random scenarios). SY1SY2 and SMP contain no census site (`census_freq.py` finds all 15 in the FX, PNO and ORG images), so the SY pass has nothing to count.

| Site (short-word pc) | Image | Shape | Executed | Count | Audio path |
|---|---|---|---|---|---|
| `0x1211C4`, `0x1211C7` | PNO `core1_sram` | `MR1F = R3, R1 = DM(..)` / `MR0F = R4, R2 = DM(..)` | **yes, every pass** | 1,500 of 1,500 passes (sharc-pass), 316 of 316 (sharc-sys) | PNO pass, the multiply-accumulate `MRF += R1 * R2` that follows |
| `0x120E01` | PNO `core1_sram` | `IF cc M12 = DM(), R0 = FPACK F0` | no | 0 | none seen |
| `0x1247AF`, `0x1247B1`, `0x1247ED` | PNO `core1_sram` | `IF cc R<->S` | no | 0 | none seen |
| `0x123AD4`, `0x123AD6`, `0x123B12` | FX `core2_sram` | `IF cc R<->S` | **yes**, once per call of the ENS (Mod 2 ensemble) routine `0x123A7B..0x123BCF` | 48 of 48 scenarios (each) | FX Mod 2 ENS |
| `0x124448`, `0x12444A`, `0x124486` | FX `core1_sram` | `IF cc R<->S` | no: core 1 copy of the ENS routine; no stage installed it in boot log, identity, reverb0, chor4, chor5 | 0 | needs an ARM recall with ENS (not run) |
| 17 pcs `0x181E60..0x182018`, `0x18253B..0x1825B5`, `0x1821C4` | ORG `core2_sram`, PNO `core2_sram` | MV/MS triple operation, conditional `LSHIFT`, `STKYx = DM (LW)` | no | 0 in boot plus 416 passes | none seen (ORG `core1_sram` was not run in a pass) |

Old against new model, same inputs (old = `main` before D151, new = this branch; release binaries):

| Run | Result |
|---|---|
| PNO core 1, Vox and Pipe, 3,000 passes, lane `2:0` WAV and all 18 raw lanes (`--raw`) | SHA-256 identical, 4 of 4 files: **bit-identical, difference 0 (-inf dB)** |
| ORG core 2, 400 passes, final SRAM data words (`--state-out`, `cmp`) | identical |
| FX core 2 ENS, 96 random scenarios against the Python model word for word, the 3 sites executed | 0 of 96 differ with the old and with the new model: **bit-identical** |
| FX core 1, 4 families, 3,000 lead passes | no site reached; new output only (identity `8b07c314`, reverb0 `47a2788d`, chor4 `63532014`, chor5 `a645f1b3`, SHA-256 prefixes) |

So the model fixes do not change a sample of any run that was made, including the runs where sites execute. The MR2F site runs every pass and is the one place where the model and the simulator could still differ; the next section shows that the differing read is not part of the firmware sequence.

## MR2F read-back resolved (D156)
`census_probe.py` on the simulator, 120 random states, `MR1F = R3, R1 = DM(-30, I0)`:

| R3 | previous MR2 bit 15 | simulator MR2F read | model |
|---|---|---|---|
| positive | 0 or 1 | `00000000` (76 cases) | `00000000` |
| negative | 0 | `0000FFFF` (28 cases) | `FFFFFFFF` |
| negative | 1 | `FFFFFFFF` (16 cases) | `FFFFFFFF` |

The simulator reads MR2 after an MR1 write with the sign extension of the previous MR2 value, not of the new one; the 16-bit MR2 value is `0xFFFF` in both. The firmware sequence `MR1F = R3 (+load)`, `MR0F = R4 (+load)`, `MRF = MRF + R1 * R2 (SSF)` gives a state identical to the model in **120 of 120** states (MR2F, MR1F, MR0F, flags and sticky bits included), because the accumulate takes MR2 as a 16-bit value. Decision: keep the sign-extending read (what `fixed_measured.rs` measured for multiplier results), pin the artefact in `census_regress.rs` (28 cases differ in exactly the MR2F word, `0000FFFF` against `FFFFFFFF`) and pin the firmware sequence (0 differ). Fixtures `census_mr2_*.txt`, `census_mr2seq_*.txt`. Whether real silicon shows the artefact is not measured (ASSUMED simulator register-image artefact).

## JIT and AOT against the interpreter (D156)
* `core/census_engines.rs` replays all 1,580 census regression cases (1,119 + 461) on the interpreter and on the Cranelift JIT (`run_jit` for the interpreter's step count): dumped state and scratch words equal in 1,580 of 1,580. The JIT lowers only `cond 31`, `ureg < 16`, non-`LW` moves and compute (`jit.rs`) and falls back to `Core::step` for shift immediates, swaps, MV/MS conditions and long-word moves, so the five semantics have one implementation.
* AOT (`aot_lower.rs`): native ops are `cond 31` compute, `LoadImm`, `RegMove` of data registers and the bit shifts `0xC0 / 0xC4 / 0xC8` (`Pe::shift`); the rest runs as interpreter items. Of the 475 distinct census instructions 3 lower natively; the 44 that are `RegSwap` or carry condition 5, 6, 21 or 22 are all interpreter items. `Pe::shift` equals the interpreter shifter (result, SZ SV SS, SF bit) for 3 opcodes x 5 operands x 10 bit numbers including 31, 32, 256 and negative ones. Generated AOT blocks come from firmware ranges, so census slot code cannot run as a block: parity is shown by classification plus the `Pe::shift` check, not by a lockstep run.

## Implications
The oracle matches the simulator on every firmware instruction form except the MR2F read-back after `MR1F = Rn`, which the firmware sequence never reads (above), so a difference between the oracle and the simulator can no longer come from single-instruction semantics. Of the 15 sites, 2 run every PNO pass (MR2F, no effect) and 3 run in the FX ENS routine (no difference); no available run reaches the others. JIT and AOT share the five semantics with the interpreter. The census runs on the **21489** simulator only: the SHARC+ (21571) behaviour of the five fixed shapes is not measured (Follow-ups). No audio output of the runs listed changes (bit-identical).

## Follow-ups
Run the census on the ADSP-21571 cycle-accurate simulator (about 4 h, `census21_fixture.txt`, `gen_census_probe.py --plus`); extend the state generator to circular buffers (L > 0) and hardware loops; resolve `SF` (no firmware use); reach the 12 sites that no available run executes (FX core 1 ENS needs an ARM recall, `arm_dsp_live.py` has no FX preset; the ORG and PNO `0x18xxxx` block, PNO `0x1247AF`, `0x120E01`) with longer ARM-driven programs; count sites in JIT and AOT runs (the counter is interpreter only).

## Reproduce
```bash
python tools/ns4_dsp_images.py target/fw_unpacked/bank0_CODE.bin --out IMG
cd docs/research/sharc-seq
python gen_census_probe.py OUT && cc21k -proc ADSP-21489 -si-revision any -g -O0 -o OUT/cens.dxe OUT/cens.asm OUT/cens.c
python dxe2fixture.py OUT/cens.dxe census_fixture.txt --code 124300 1245D0 --data B2000 B2001 --entry 124300 --exit 1245CE --out B206C 107 \
  --sym _slot 124462 --sym _st B2001 --sym _res B206C --sym _scr B20D7 --sym _done B2000   # addresses from elfsyms
python census_diff.py IMG OUT/cens.dxe census_fixture.txt --work WORK --jobs 16
python census_freq.py IMG --list      # occurrences per shape (counts and addresses)
python census_probe.py OUT/cens.dxe census_fixture.txt --work W2 --parcels-file LIST --states 6 --keep NAME
python census_regress.py IMG OUT/cens.dxe census_fixture.txt --work WORK     # rebuilds census_cases/measured
```
