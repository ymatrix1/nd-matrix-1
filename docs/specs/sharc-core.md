# SHARC ADSP-214xx core model (Track B oracle)

Specification for a test-oracle core that executes single routines of the real DSP images. It is **not** a full-system emulator and it does not replace the high-level engine ([D2](../DECISIONS.md)); the one opt-in exception is the minimal system run of section 7 ([D119](../DECISIONS.md#d119-the-sharc-oracle-becomes-the-dsp-half-of-a-minimal-system-run-opt-in-org-core2-only-no-default-change), proposed). A proposed follow-on phase to promote this core into a unified real-time ARM + SHARC low-level emulator (LLE), gated on real-time speed benchmarks, is tracked in Phase 8 (`ns4-lle1`, [ROADMAP](../ROADMAP.md)). Scope and counts come from `tools/sharc_census.py` (`docs/research/dsp-image-hunt.md` 5.19).

## 1. Goal

Run a routine of the FX, PNO, ORG, SY1SY2 or SMP image on test vectors and compare its output with the Rust engine, so that engine constants and algorithms can be checked against the firmware. Typical use: load a core's SRAM image, set registers and memory, call a routine address, run until it returns, read the result.

## 2. Inputs

| Input | Source |
|---|---|
| Per-core SRAM image | `tools/ns4_dsp_ldr.py --sram` (flat image indexed by `address & 0x7FFFFF`, byte `2 * short_word_address`); mapping VERIFIED, `dsp-image-hunt.md` 5.18 |
| Entry value | section header of the loader stream (short-word address) |
| Instruction encoding | SHARC Processor Programming Reference rev 2.4, chapters 9 to 12 (`.reference/`, not committed); decoder in `tools/sharc_visa.py` |
| Memory map | ADSP-21489 datasheet Table 5 (internal blocks, short, normal and 48-bit aliasing) |

## 3. State to model

- Register file: `R0..R15` / `F0..F15` (PEx) and `S0..S15` (PEy, SIMD), 40-bit data with the float extension, `MRF/MRB/MSF/MSB` multiplier accumulators.
- Data address generators: `I0..I15`, `M0..M15`, `L0..L15`, `B0..B15` (circular buffers, bit-reverse), pre-modify and post-modify addressing.
- Sequencer: `PC`, PC stack, loop stack (`LCNTR`, `CURLCNTR`, `DO ... UNTIL LCE`), delayed branches, status stack, `MODE1`, `MODE2`, `ASTAT`, `STKY`, `USTAT1..4`, `IMASK`, `MMASK`, `IRPTL`.
- Memory: four internal SRAM blocks as one byte array with short-word, normal-word (32-bit) and instruction-word (48-bit) views; the external delay-line window (`0x20xxxxxx` loader addresses, `0x0800_0000` data addresses) as a dense word array (`core/sdram.rs`).

## 4. Instruction scope (from the census)

Implement first, in this order:

1. Moves and data memory: `Ureg = Ureg`, `Ureg = <imm>` (types 17a/17b), `DM/PM(imm, Ii)`, `DM/PM(Ii, Mm)` with pre and post modify, direct `DM(addr)`. About 21% of all census instructions are memory moves.
2. Float compute: multiply (13,515 plain, 383 with modifiers), add (9,314), subtract (4,372), `PASS`, `ABS`, `CLIP`, `COPYSIGN`, `MAX/MIN`, `(a+b)/2`, `RECIPS`, `RSQRTS`, `FLOAT`, `FIX`, `COMP`; multifunction forms (multiplier plus ALU, dual ALU).
3. Control: `DO UNTIL LCE`, `CALL`, `JUMP`, `RTS`, conditional execution (`IF cond`), `NOP`, `IDLE`.
4. Bit and integer: `AND/OR/XOR`, `FDEP/FEXT`, `LSHIFT/ASHIFT`, `MODIFY`, `BIT SET/CLR/TST` on system registers.
5. SIMD: PEy registers, `Dreg <-> CDreg` swap, the `MODE1` PEY enable bit.

Instruction decoding for all used forms is a prerequisite. `tools/sharc_visa.py` currently decodes the type for 98.7% and the operands for 95.4% of the instructions in the large blocks. The remaining forms (1a, 7a/7b, 9a/9b, 10a, 16a/16b, parts of 15b) must be completed first.

## 5. Milestones and acceptance

| Milestone | Acceptance |
|---|---|
| M1 decoder | Full operand decoding into a Rust enum; round trip against the `tools/sharc_visa.py` listings on all code spans. **Status:** `crates/nord-sharc/src/visa/` (submodules `types`, `classify`, `regs`, `compute`, `decode`) ports the Python decoder (type, length and text for every form the Python tool decodes) and its `sharc-disasm` output is identical to the Python listing on about 970,000 instruction lines of four core SRAM images; still open: structured operands (an enum) for the execution core and the forms the Python tool leaves raw |
| M2 core | Registers, DAGs, loops and calls; one unit test per instruction from the semantics in the manual  **Status:** `crates/nord-sharc/src/core/` (submodules `memory`, `dag`, `alu`, `multiplier`, `moves`, `branch`, `exec`) implements moves, DM/PM addressing with circular buffers, `MODIFY`, `Dreg <-> CDreg` swaps, system-register bit operations, jumps and calls with delay slots, `DO UNTIL LCE` loops, the single-function ALU and short compute (float and fixed) and the fixed-point multiplier with MRF/MRB; SIMD (`PEy` registers `S0..S15`, per-element flags and conditions, data transfers at `addr + 1`, immediates broadcast, MODE1 bit 21), the barrel shifter (`shifter.rs`: shifts, rotates, single-bit ops, `FDEP`/`FEXT`, `EXP`, `LEFTZ`/`LEFTO`), multifunction compute (`multifunction.rs`: multiplier plus ALU, triple op, dual add/subtract) and `SAT`/`RND` of MRF/MRB are modelled; interrupts, DMA, shift immediate (Type 6a), `FPACK`/`FUNPACK`, shift immediate (`0x02` form) and the bit FIFO (`bitfifo.rs`: `BITDEP`, `BITEXT`, `BFFWRP`, `IF SF`; per-PE state; [findings](../research/sharc-bitfifo.md)) are modelled; DMA faults explicitly; **Sequencer** (`interrupts.rs`, `vectors.rs`, `timer.rs`, `sysregs.rs`, `stacks.rs`, `banks.rs`; [research](../research/sharc-sequencer.md)): interrupt controller with IVT dispatch, nesting, `RTI`, `IDLE`, status/PC/loop stacks with depth limits, core timer, `EMUCLK`, ASTAT/STKY/FLAGS, secondary register sets, `CBUFEN`, bit-reversed addressing, Type 19a; `sharc-run --irq N@STEP --flag N=LEVEL`; **Sequencer leftovers** (`arith_loop.rs`, `pcstack.rs`, `branch_ext.rs`, `dag_modes.rs`; [research](../research/sharc-leftovers.md)): Type 13a `DO ... UNTIL cond`, `PUSH/POP LOOP|STS|PCSTK` with `PCSTK`/`LADDR`/`CURLCNTR` writes, `JUMP (LA)`/`(CI)`, `RTS (LR)`, 48-bit delayed `RTS/RTI` with compute, `CJUMP`/`RFRAME`, `BDCST1/9` broadcast loads, `CB7I`/`CB15I`; float ALU/multiplier fidelity (40-bit extended precision, TRUNC/RND32, denormal/NaN/INF rules, `RECIPS`/`RSQRTS`/`MANT`/`LOGB`, AI/AS flags) is modelled in `float*.rs`, seed tables ASSUMED, see `docs/research/sharc-float.md` (`sharc-run --lenient` skips and counts compute faults, `--skips` lists them) |
| M3 start-up | Run the entry code of each core section until it reaches its idle loop with no unknown instruction. **Status: done for FX Core1/Core2 and ORG Core1/Core2** (`dsp-image-hunt.md` 5.21): each reaches its main loop (peripheral poll or command mailbox); PNO is not run yet **SY1SY2 and SMP** (`research/sharc-sy1sy2-smp.md`): the ISA-mode boot kernel (`sharc-run --isa`, now to its final `IDLE`, which waits for a DMA the oracle does not model) and both programs run their start-up to a `JUMP (PC, 0)` idle loop once FLAG0 is high (`--flag 0=1`); their work is interrupt-driven (`--enter STEP:PC` enters a handler body). **ORG processing pass:** `sharc-org` (`crates/nord-sharc/src/bin/sharc_org.rs`) boots a section, runs scripted passes with data pokes and reports diffs, store sites, calls and audio buffers ([findings](../research/org-dsp-map.md)). **Reference render:** `sharc-ref` (`crates/nord-sharc/src/bin/sharc_ref.rs`) runs a scenario file (pokes, signal fills, voice records, handler runs, captures) and dumps CSV/WAV for `tools/ns4_oracle_compare.py`; first use: the SY1SY2 Stage B resampler, bit-exact against the 40-bit arithmetic rules ([oracle-vs-engine](../research/oracle-vs-engine.md)). |
| M4 routine test | Call one pure routine (for example a biquad or the sinc interpolation) with test vectors and match the expected result |
| M5 host protocol | Reverse enough of the ARM-to-DSP messages (`DSP Comm`, `bank0_CODE@0x6CA48`, more than 1,000 call sites) to set one effect's parameters |
| M6 engine comparison | Render an effect through the oracle and compare it with the Rust engine |

## 6. Risks

- Float exactness: 40-bit extended precision and the rounding modes (`RND32`, `TRUNC`) are modelled (`docs/research/sharc-float.md`); the `RECIPS`/`RSQRTS` seed ROMs and their flags are VERIFIED against the CCES 2.12.1 ADSP-21489 simulator (`float_seeds.rs`, `docs/research/sharc-seeds/`); `FIX`/`TRUNC`, rounding and the 40-bit extension are VERIFIED on the same simulator (`float_measured.rs`, `docs/research/sharc-round/`); special values, ALU/multiplier flags, STKY and the S-file extension are VERIFIED the same way (`float_flags.rs`, `docs/research/sharc-flags/`); CACC is modelled. The fixed-point ALU, multiplier/MR, shifter flags and CACC are VERIFIED on the same simulator (`fixed_measured.rs`, `docs/research/sharc-fixed.md`); MRB, the SRCU MR exchange, the integer `R` multiplier forms, the shifter OR forms and the FIX sticky bits are VERIFIED by a second probe set; SIMD (PEy) compute, ASTATy/STKYy, AF, the dual add/subtract flags and NaN multiplies are VERIFIED too (`simd_measured.rs`, `docs/research/sharc-simd.md`); SIMD loops stay open. Every single-instruction form of the five DSP images was then run on the simulator in six random core states and compared with the model (`docs/research/sharc-census-diff.md`: 12 model gaps fixed, 18 of 49 722 cases left open, 1 104 replayed by `census_regress.rs`). Tolerance-based comparison is expected; bit-exactness is a stretch goal.
- Peripherals: only the register blocks the images use are modelled, in `core/periph/` and off by default (a `--dev` preset always wins). **FX, PNO and ORG run on an ADSP-2157x** (SHARC+ dual core, `0x3xxxxxxx` registers, L2 at `0x20000000`), **SY1SY2 and SMP on an ADSP-2148x** ([sharc-chip-identity.md](../research/sharc-chip-identity.md)). `--sc57x CORE` enables SEC0 (CSID/END, done interrupts to `P4I`), SPI2, descriptor-list DMA with TRU triggers, LP1 and 1 MiB L2 (`--l2 ADDR:FILE`, files from `ns4_dsp_ldr.py --sram`; `--spi2-rx` words, `--sec-raise ID`); the ORG Core1 init then completes and the SPI2 peek port answers. `--spi-boot IMAGE[:OFFSET]` serves the 2148x SPI-slave DMA (`IISPI/IMSPI/CSPI/SPIDMAC`, completes when the program polls or sleeps) so the boot kernel loads the whole block stream and reaches the program idle loop (SY1SY2 `0x1C0B33`, SMP `0x1C11FD`). Register offsets are VERIFIED against CCES headers, timing and side effects are ASSUMED until measured on the simulator. SPORT and audio DMA registers are still unmodelled (interrupts come from `Core::raise_interrupt` / `--irq`); the inter-chip audio path is modelled by its effect, s10.
- Long-word accesses: `Rn = DM(..) (LW)` / `DM(..) = Rn (LW)` move the register pair `Rn, Rn+1` (VERIFIED against ADI elfdump, `core/longword.rs`); the SY1SY2 frame clear and 64-bit voice masks depend on it. SIMD + LW and Type 4 forms are not modelled (`docs/research/sy-smp-audio.md` section 6).
- Interrupt-driven scenarios: `sharc-ref` raises vectors through the interrupt controller (`irq`) and feeds the SY/SMP command FIFO (`msg`), so the audio ISR runs with the firmware's own register banks (`docs/research/sy-smp-audio.md`).
- ISA mode: with an ISA image loaded, a `pc` below `0x100000` is an ISA word address, above it VISA short words; interrupt vectors come from the ISA table at `0x8C000 + 4 * vector`; the data window `0x100000..0x1FFFFF` is 16-bit short words of the same memory.
- External memory: delay and reverb lines live in the window `0x0800_0000..0x0804_0000` (normal-word addresses; loader byte address `0x2000_0000 + 4 * (w - 0x0800_0000)`, VERIFIED on the FX Core1/Core2 zero blocks), modelled as a dense 1 MiB word store (`core/sdram.rs`, always on, counters `sdram_reads` / `sdram_writes`, [sharc-sdram-window](../research/sharc-sdram-window.md)). The physical part and latency are not modelled (ASSUMED plain memory).
- Provenance: the oracle is only as good as the decoding; every newly decoded field needs a test that fails without it.

## 7. Minimal system run (D119, proposed, opt-in)

The oracle can act as the DSP half of a small system run: the ARM firmware runs in the Unicorn harness (`tools/arm_org_stream.py`), its ORG Core2 messages are replayed between interpreter passes by `sharc-sys` (`crates/nord-sharc/src/runner/system.rs`), and every component of a pass interval is timed. Scope: ORG Core2, interpreter, one core.

| Part | State |
|---|---|
| ARM to DSP messages | modelled by effect: `DM(word) = value` before the named pass (`--msg-script`, `--script-flat`); link latency and the 3-word SPI frame are not timed |
| Pass trigger | mailbox `DM(0xB00E2)` write, as in `sharc-org`; PNO core 1 P4I vector 15, optionally through the SEC model (`--sec-id`, D120) |
| Audio-input DMA | stub only: the pass-input words `DM(0xB0078..)` and `DM(0xB00C8..)` are re-stored (cost, no data) |
| External-port range registers `DM(0x3E000..0x3E03F)` | opt-in `--epdma` (`core::periph::epdma`): a range opens when `DM(0x3E010)` is written with `DM(0x3E002)` bits 1:0 set, `DM(0x3E021)` returns the opened count and clears it (ASSUMED, D120); replaces `--pre 0x3E021=1`, 0 differing words |
| Peripheral census | opt-in `--census` (`core::periph::access_log`): every access outside SRAM with counts and last value |
| Leslie history-ring restart | opt-in `--ring-dma` (`core::periph::ringdma`, D126 placeholder): `DM(0x310C3000)` `7` then `0x107` copies the first record (`0x40` words) of chains `0xE31E8` and `0xE35E0` in `0x0A00_0000` DMA view into `c[-8]` (`0xE31A6` / `0xE359E`); `sharc-ring` (`runner/ring_trace.rs`) traces load/store windows per pass |
| Data tap | `--tap ADDR:COUNT` (repeatable) with `--tap-out FILE`: those `DM` words after every pass, little-endian `u32`, pass-major (`runner/tap_run.rs`); reads stage records that reach no lane (D132) |
| PNO output lane | `--lane SET:PAIR`, `--wav`: reads the output pointer set after each pass (stands in for the SPORT transmit DMA); needs the boot message log |
| Interrupts, other cores, SPI/SPORT | not modelled |

Opt-in additions: `sharc-sys --jit-diff` (`runner/system_diff.rs`) runs the same passes on the interpreter and the JIT in lockstep and stops at the first divergence; `--irq VEC:STATUS` and `--flag1` start Core1 sections by the receive interrupt. Measurements for PNO core 1, the ARM tick cost and the other cores: [pno-jit-arm-profile](../research/pno-jit-arm-profile.md).

Evidence, timing table and limits: [org-system-run](../research/org-system-run.md); peripheral census, `0x3E0xx` rule, PNO lane and hardware numbers: [dsp-peripherals-audio](../research/dsp-peripherals-audio.md); B3 Leslie ring-DMA trace and lane check: [b3-leslie-pno-lane](../research/b3-leslie-pno-lane.md).

## 8. FX chip live run (D125, proposed, opt-in)

`sharc-fxlive IMAGE --core 1|2` (`crates/nord-sharc/src/runner/fx_run.rs`, new bin) runs the FX Core1 or Core2 pass on a stereo signal: messages (`--msg`, `--script-flat`) are delivered before the named pass, 4 frames per pass are written into the input block (`--slot`, default `0xC01BC` core 1, `0xC0180` core 2; `--in-wav`, `--sine`, `--impulse`, `--lead N` silent passes because the master stage fades in over about 2000 passes), the pass runs from the `P4I` vector 15 (core 1) or the mailbox (core 2), and the output frames are collected (`--wav`). `--tap ADDR` records an 8-word bus after every pass, `--regs-at PC` prints the `M` and `I` registers and `DM(I0..+8)` where a pass first reaches `PC` (entry registers of a stage routine), `--census` lists accesses outside SRAM. Flags off: `sharc-sys`, `sharc-fx` and every other bin are unchanged (one `start()` visibility change in `runner/system.rs`).

| Item | State |
|---|---|
| Chain transfer, pass-through stage `0x1231CB` | output = `0.1866 x DM(0xB6518) x input`, lag 0, -157.6 dB null after the gain fit (VERIFIED, [fx-oracle-live](../research/fx-oracle-live.md)) |
| Stage install | records `S-0x20 ..` written by messages as the ARM does; reverb type 0 (`0x124AF2`) and Mod 2 CHOR (`0x123798`) install and run, the dispatcher calls an installed head stage 35 times per pass unless the head link and nine group headers are pointed at a null stub record (`null_chain()` in `tools/fx_oracle_live.py`, D133); with that, reverb type 0 and CHOR 4 / 5 (rate 64, amount 64) match the native ports |
| Native counterpart | `fx_live_native` (`crates/nord-tools`), `tools/fx_oracle_live.py` compares (lag, fitted gain, RMS delta, centroid, tail, null depth) |
| PNO lane to FX input | wired by the audio bus, s10 (the WAV hand-over stays available) |

## 9. Whole-Pass AOT and multi-core pass benchmark (`ns4-lle3`, opt-in, not real time)

| Item | Value |
|---|---|
| Build | `NORD_SHARC_AOT_HOT=TAG=file;TAG2=file2` embeds generated whole-pass modules (`sharc-aot --whole-pass PCS --force IDLE_PC --tier pe\|approx`); `Core::enable_aot_tag(tag)` installs one after a code check. Without the variable a stub is built (CI stays firmware-free) |
| Lowering | `core/aot_mem.rs` `MemXfer`: DirectMem, DagMem, SIMD twin and broadcast, Move/RegSwap, ShiftImm, Multifunction; conditional and long-word forms stay on `Core::step`. The generated module calls one out-of-line handler per transfer shape (`hN(core, pc, imm)`, `core/aot_emit.rs`); compute runs in place on the core's registers (`Pe<'a>`, `core/aot_pe.rs`); an instruction that may change guarded state (system register write, bit operation) ends its block (`aot_whole.rs`, `Guard::Cut`), a store re-checks the guard only after an access outside the SRAM (`Core::aot_slow`) |
| Event-aware blocks (D258, D1) | `sharc-aot --ev` emits blocks whose items compare `steps` with the live `irq.due` (handler head, generic step, interpreter item; one pre-check per native batch). A due event runs that item (the whole batch) on the interpreter, which takes the event; the block returns the interpreter's exact state through `aot2::events::SPLIT` when an interrupt was taken (`irq.taken`) or the pc left the straight line, and carries on natively otherwise. `AotBlock::ev` drops `steps + n <= due` from the entry guard; `aot_sig_ev` leaves `due` out of the signature; `aot_slow` is set after an event item so a following store check re-reads the signature. Blocks without `--ev` are v1, byte-identical, the oracle. Counters: `AotHotStats::{ev_steps, splits, taken}` |
| Exactness | VERIFIED by `aot_mem` random-state tests, `aot2/ev_blocks.rs` (hand-built event-aware blocks equal the interpreter with an event at every position on a delayed branch, a loop end and a taken interrupt) and `sharc_jit_diff --aot-top`: ORG Core2, 1 854 395 steps 1:1, strict state every 20 000 steps now including `imask`, `irq.taken/stall/idle/window_len` and `pending`. `NORD_SHARC_AOT_TAG=TAG` selects the embedded module the diff runs |
| Scheduler | `runner/pass_sched.rs`, binary `lle_sharc_bench` (`nord-tools`): one thread per core, atomic spin barrier |
| Result | ORG Core2 x3.5 over 46.08 us/pass with AOT (x7.2 before the bounded experiment, x4.6 after it, x3.5 after lowering v2 stages A and B; fast-f32 x2.93; interpreter x10 to 13), PNO Core2 x6.7 on the interpreter: NO-GO for real time in exact mode, `docs/research/sharc-lle-realtime.md`. D1 (D258): refused entries 14 to 0 per pass; wall-clock not shown to change (loaded host) |

## 10. Multi-chip audio bus, FX hand-over and delay window (D247, `ns4-lle4`, opt-in, not real time)

| Item | Value |
|---|---|
| FX hand-over | `FxDrive::before_pass` presents the Core1 counter (Core2) and the ping selector before every pass; `PassJob::before_pass` is the one pre-pass step (ping-pong, messages, FX drive, input refresh) of `run_ticks` and `CoreNode`. Presets `fx1`, `fx2` in `live_presets.rs`. FX Core1/Core2 run 316 passes idle at 3 370 / 3 220 steps (VERIFIED, [sharc-audio-bus](../research/sharc-audio-bus.md)) |
| Delay window | `core/sdram.rs`: dense `0x0800_0000..0x0804_0000`, `loader = 0x2000_0000 + 4 * (w - 0x0800_0000)` VERIFIED on the FX zero blocks, plain memory ASSUMED ([sharc-sdram-window](../research/sharc-sdram-window.md)) |
| Bus | `runner/audio_bus.rs`: `BusNode` (frames in per pass, `every` ticks, frames out), `Route {from, to: Node\|Host, gain}`, per-node input FIFO, one tick per hop, short input zero padded and counted, routes into one input add. Nodes: `CoreNode` (ORG/PNO lane, FX block in / master words out), `SyNode` (SY1SY2/SMP `SyModel`), `lle_system.rs` builds them from `NAME=IMAGE,PRESET[,MSGFILE]` |
| Runner | `lle_sharc_bus --node ... --route FROM>TO[:GAIN] --in-wav NODE:FILE --ticks N --wav OUT` (nord-tools), one thread, deterministic |
| Exactness | FX Core1 behind the bus equals `sharc-fxlive --core 1` byte for byte (17 000 passes, `identity` stage, `cmp` of the WAVs) |
| Open | tone-chip to FX topology and DMA depth (ASSUMED, caller data), real tone audio through installed FX chains (needs ARM key/effect messages), frame exchange inside the threaded scheduler, `ns4-36hw` gaps (not hit) |
