//! The start state of a fuzz case: registers, hidden state, DAG registers with live circular
//! buffers, multiplier extremes, memory around the indices and the interrupt table.
//!
//! OWNER: W3-FUZZ. The generic fillers are `verify::gen` (`fill_state`, `fill_stale_ext`,
//! `fill_sticky`, ...); the DAG and the memory are the domain of the move family and are filled
//! here (design P13: a family's domain is filled by its tests, not by `fill_state`). Everything
//! is a function of one seed, so the interpreter core and the engine core get the same state.
use super::sieve::{DATA, DATA_SPAN};
use crate::core::interrupts::{IRPTEN, NESTM, VECTOR_PARCELS};
use crate::core::support::load;
use crate::core::uop::verify::gen::{
    edge_word, fill_astat_extra, fill_fifo, fill_stacks, fill_stale_ext, fill_state, fill_sticky,
    Rng, CODE_PC,
};
use crate::core::Core;

/// MODE1 bits of the move domain: bit-reversed `I8` and `I0` (bits 0 and 1), broadcast `I9` and
/// `I1` (22 and 23) and the circular-buffer enable (24). VERIFIED: the same mask the `h_move`
/// tests use for these bits (`dag.rs`, `dag_modes.rs`).
const MODE1_DAG: u32 = 1 | 1 << 1 | 1 << 22 | 1 << 23 | 1 << 24;

/// Which parts of the hidden and addressing state a case fills (each one is a different corner
/// of the exactness contract; a family that does not touch a part keeps it zero).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Prep {
    /// PEy enable.
    pub simd: bool,
    /// Float extensions of an older register value (exact_diff compares raw extension bytes).
    pub stale: bool,
    /// Sticky bits, ASTAT extras and the shifter FIFO.
    pub hidden: bool,
    /// DAG registers, live circular buffers, MODE1 address bits and the data memory.
    pub dag: bool,
    /// Index registers may point into the code (self-modifying code).
    pub into_code: bool,
    /// Multiplier registers at the edges of the 80-bit range.
    pub mr_edges: bool,
    /// Return addresses on the call stack.
    pub stack: usize,
}

impl Prep {
    /// Everything on: the busiest start state a case can have.
    pub fn full(simd: bool) -> Prep {
        Prep {
            simd,
            stale: true,
            hidden: true,
            dag: true,
            into_code: false,
            mr_edges: true,
            stack: 2,
        }
    }
}

/// An address of the kinds a transfer meets, SRAM data first. `into_code` adds the code words
/// themselves, a third of the time (a store there is self-modifying code).
fn pick_addr(rng: &mut Rng, into_code: bool) -> u32 {
    if into_code && rng.below(3) == 0 {
        return CODE_PC / 2 + rng.below(24); // a word of the program itself
    }
    match rng.below(20) {
        0..=10 => DATA + rng.below(DATA_SPAN),
        11 | 12 => DATA + rng.below(8),   // the low edge of the data
        13 => 0xF_FFFC + rng.below(4),    // top of the SRAM window: `+ 1` leaves it
        14 => 0x3_E000 + rng.below(8),    // IOP array
        15 => 0x12_4000 + rng.below(8),   // short-word alias of SRAM
        16 => 0x0802_0000 + rng.below(8), // SDRAM delay-line window
        17 => 0x70_0000 + rng.below(8),   // device store
        _ => [0x3108_D004, u32::MAX, 0, 0x1081][rng.below(4) as usize], // periph, wrap, low
    }
}

/// Fills `I`, `M`, `L`, `B`: half of the registers become live circular buffers (`L` 2 to 8, `B`
/// inside the data, `I` inside the buffer, so a post-modify wraps within a few steps), the others
/// plain indices with modifiers at the boundaries (0, 1, 2, 3, -1, -2, 7, 0x100). Then the data
/// memory around the indices, the IOP words and the device words are written through the
/// interpreter's own write path, so both engines see the same values and the same side counters.
fn fill_dag(c: &mut Core, rng: &mut Rng, into_code: bool) {
    for k in 0..16 {
        c.i[k] = pick_addr(rng, into_code);
        c.m[k] = [0, 1, 2, 3, u32::MAX, u32::MAX - 1, 7, 0x100][rng.below(8) as usize];
        if rng.below(2) == 0 {
            c.l[k] = 2 + rng.below(7);
            c.b[k] = DATA + 0x80 + rng.below(0x20);
            c.i[k] = c.b[k] + rng.below(c.l[k]);
        } else if rng.below(4) == 0 {
            (c.l[k], c.b[k]) = (0, pick_addr(rng, false));
        }
    }
    // the I7 and I15 wrap latch is the interesting one: keep one of them at the end of a buffer
    let k = if rng.below(2) == 0 { 7 } else { 15 };
    (c.l[k], c.b[k], c.m[k]) = (4, DATA + 0x90, 1);
    c.i[k] = c.b[k] + 3;
    c.mode1 = (c.mode1 & !MODE1_DAG) | (rng.next_u32() & MODE1_DAG);
    let words = (DATA - 8..DATA + DATA_SPAN)
        .chain(0xF_FFF8..0x10_0000)
        .chain(0x3_E000..0x3_E008)
        .chain(0x70_0000..0x70_0008);
    for w in words {
        let _ = c.dm_write(w, rng.next_u32());
    }
}

/// Multiplier registers at the edges of the 80-bit accumulator: the largest and smallest values,
/// one bit either side of the 64-bit and 32-bit boundaries and the sign extension of each.
/// ASSUMED: these are the boundaries where the saturation and the spill into the upper bits
/// differ; a boundary this list misses shows as a gap in the family fuzz, not as a wrong claim.
fn fill_mr_edges(c: &mut Core, rng: &mut Rng) {
    const EDGES: [i128; 12] = [
        (1 << 79) - 1,
        -(1 << 79),
        (1 << 63) - 1,
        -(1 << 63),
        1 << 63,
        -(1 << 63) - 1,
        (1 << 31) - 1,
        1 << 31,
        -(1 << 31),
        -1,
        0,
        1,
    ];
    for m in c.mr.iter_mut().chain(c.mr_y.iter_mut()) {
        *m = EDGES[rng.below(EDGES.len() as u32) as usize];
    }
    // the fixed-point registers hold edge words too, so a saturating op meets both sides
    for r in c.r.iter_mut() {
        if rng.below(2) == 0 {
            *r = edge_word(rng) ^ (0x8000_0000 * rng.below(2));
        }
    }
}

/// Fills `c` from `seed`: the same seed and `prep` give the same state on every core.
pub(super) fn prepare(c: &mut Core, seed: u64, prep: &Prep) {
    let mut rng = Rng::new(seed);
    fill_state(c, &mut rng, prep.simd);
    if prep.stale {
        fill_stale_ext(c, &mut rng);
    }
    if prep.hidden {
        fill_sticky(c, &mut rng);
        fill_astat_extra(c, &mut rng);
        fill_fifo(c, &mut rng);
    }
    if prep.dag {
        fill_dag(c, &mut rng, prep.into_code);
    }
    if prep.mr_edges {
        fill_mr_edges(c, &mut rng);
    }
    if prep.stack > 0 {
        fill_stacks(c, &mut rng, prep.stack, 0);
    }
}

/// Aims the indirect jumps and the returns at instruction starts: `I8..I15` plus `M8..M15` add up
/// to a start, the circular buffers of DAG2 are off, and the call stack holds up to three starts.
/// The draw is a function of `seed` alone.
pub(super) fn aim_at(c: &mut Core, seed: u64, starts: &[u32]) {
    let mut rng = Rng::new(seed ^ 0xA1_4E_D0);
    let start = |rng: &mut Rng| starts[rng.below(starts.len() as u32) as usize];
    for k in 8..16 {
        c.m[k] = rng.below(4);
        c.i[k] = start(&mut rng).wrapping_sub(c.m[k]);
        (c.l[k], c.b[k]) = (0, 0);
    }
    c.truncate_returns(0);
    for _ in 0..rng.below(4) {
        let at = start(&mut rng);
        c.push_return_address(at);
    }
}

/// Short-word address of the vector table of a case with interrupts. ASSUMED: any place away from
/// the code and below the SRAM code window does (the `h_flow` tests use the same).
pub(super) const IVT: u32 = 0x4000;
/// Short-word address of the service routine.
pub(super) const ISR_PC: u32 = CODE_PC + 0x400;
/// Highest vector with a slot in the table.
const VECTORS: u32 = 42;

/// Enables interrupts, unmasks every source and fills the whole vector table with a jump to the
/// service routine at [`ISR_PC`]. The slot is read in ISA parcel order (least significant parcel
/// first), like the `h_flow` tests write it. Nesting stays off, so one service runs at a time.
pub(super) fn arm_interrupts(c: &mut Core, arith: bool) {
    c.set_ivt_base(IVT);
    let w = 0x063e_0000_0000u64 | u64::from(ISR_PC);
    for v in 0..VECTORS {
        let at = IVT + VECTOR_PARCELS * v;
        load(c, at, &[w as u16, (w >> 16) as u16, (w >> 32) as u16]);
    }
    c.mode1 = (c.mode1 | IRPTEN) & !NESTM;
    // IRPTL bits 23..=26 are the arithmetic exception vectors 33..=36 (`vectors::slot`): left
    // enabled, a random state raises one every few steps (the `Arith` guard row refuses every
    // block that can) and no case would get past the interpreter
    c.imask = if arith { u32::MAX } else { !(0xF << 23) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::compare::exact_diff;

    fn built(seed: u64, prep: &Prep) -> Core {
        let mut c = Core::new(&[]);
        prepare(&mut c, seed, prep);
        c
    }

    #[test]
    fn one_seed_gives_one_state_on_every_core_and_each_part_changes_it() {
        let full = Prep::full(true);
        assert_eq!(exact_diff(&built(5, &full), &built(5, &full)), None);
        assert_ne!(exact_diff(&built(5, &full), &built(6, &full)), None);
        let base = Prep::default();
        for (name, part) in [
            (
                "stale",
                Prep {
                    stale: true,
                    ..base
                },
            ),
            (
                "hidden",
                Prep {
                    hidden: true,
                    ..base
                },
            ),
            ("dag", Prep { dag: true, ..base }),
            (
                "mr",
                Prep {
                    mr_edges: true,
                    ..base
                },
            ),
            ("stack", Prep { stack: 2, ..base }),
        ] {
            let d = exact_diff(&built(9, &base), &built(9, &part));
            assert!(d.is_some(), "{name} left the state alone");
        }
        assert!(built(1, &full).simd() && !built(1, &Prep::full(false)).simd());
    }

    #[test]
    fn the_dag_has_live_circular_buffers_and_the_wrap_latch_register_is_at_the_edge_of_one() {
        let mut live = 0;
        for seed in 1..=20 {
            let c = built(
                seed,
                &Prep {
                    dag: true,
                    ..Prep::default()
                },
            );
            live += (0..16).filter(|&k| c.l[k] > 1).count();
            let k = if c.l[7] == 4 && c.b[7] == DATA + 0x90 {
                7
            } else {
                15
            };
            assert_eq!((c.l[k], c.i[k] - c.b[k], c.m[k]), (4, 3, 1), "seed {seed}");
        }
        assert!(live > 100, "{live} circular buffers over 20 seeds");
    }

    #[test]
    fn indices_stay_out_of_the_code_unless_the_case_asks_for_it() {
        let hits = |into_code: bool| {
            (1..=30u64)
                .map(|s| {
                    built(
                        s,
                        &Prep {
                            dag: true,
                            into_code,
                            ..Prep::default()
                        },
                    )
                })
                .map(|c| {
                    (0..16)
                        .filter(|&k| (CODE_PC / 2..CODE_PC / 2 + 16).contains(&c.i[k]))
                        .count()
                })
                .sum::<usize>()
        };
        assert_eq!(hits(false), 0);
        assert!(hits(true) > 0);
    }

    #[test]
    fn the_interrupt_table_jumps_to_the_service_routine_from_every_vector() {
        let mut c = Core::new(&[]);
        arm_interrupts(&mut c, true);
        assert!(c.imask == u32::MAX && c.mode1 & IRPTEN != 0 && c.mode1 & NESTM == 0);
        for v in [0u32, 11, 41] {
            let at = IVT + VECTOR_PARCELS * v;
            assert_eq!(
                [c.parcel(at + 2), c.parcel(at + 1), c.parcel(at)],
                [0x063e, (ISR_PC >> 16) as u16, ISR_PC as u16],
                "vector {v}"
            );
        }
    }

    #[test]
    fn aiming_puts_the_indirect_jumps_and_the_returns_on_the_given_starts() {
        let starts = [CODE_PC, CODE_PC + 7, CODE_PC + 20];
        let mut c = Core::new(&[]);
        prepare(&mut c, 3, &Prep::full(false));
        aim_at(&mut c, 3, &starts);
        for k in 8..16 {
            assert!(starts.contains(&c.i[k].wrapping_add(c.m[k])), "I{k}+M{k}");
            assert_eq!(c.l[k], 0);
        }
        assert!(c.return_addresses().len() <= 3);
        assert!(c.return_addresses().iter().all(|a| starts.contains(a)));
        let mut again = Core::new(&[]);
        prepare(&mut again, 3, &Prep::full(false));
        aim_at(&mut again, 3, &starts);
        assert_eq!(exact_diff(&c, &again), None);
    }

    #[test]
    fn the_multiplier_edges_reach_both_ends_of_the_range() {
        let mut max = i128::MIN;
        let mut min = i128::MAX;
        for seed in 1..=30 {
            let c = built(
                seed,
                &Prep {
                    mr_edges: true,
                    ..Prep::default()
                },
            );
            for m in c.mr.iter().chain(&c.mr_y) {
                (max, min) = (max.max(*m), min.min(*m));
            }
        }
        assert_eq!((max, min), ((1 << 79) - 1, -(1 << 79)));
    }
}
