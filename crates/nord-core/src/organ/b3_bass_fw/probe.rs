//! Test-only probes of the compiled Bass generator (research s20, D199): which work could be
//! skipped or vectorised without changing one state bit?
//!
//! `run_probed` runs a compiled routine item by item (the same `run_item` as the compiled programs) and, for every pass of every hardware loop, compares the machine before and after.
//! The tests below record the two facts that closed the "unroll the loops" and "skip the voices
//! that are not held" ideas of D198:
//!
//! - no pass of the 22 and 10 pass loops of the generator is an identity on the registers,
//!   accumulators, flags and DM, with three keys held or with none, so a pass cannot be dropped
//!   and replaced by its index register effect (the loops are lattice sections whose states move
//!   every call);
//! - the float add and subtract items rarely have a zero operand in the generator (so a lane wise
//!   zero branch does not save the work of a SIMD add), while the multiplies often do.

use super::compile::{compile, ctl, Program, ALUK};
use super::image::BassImage;
use super::tables::alu_op as A;
use super::{real_image, Machine, GEN_PRESET, PROC_PRESET};

/// What the probe counted over the calls it ran.
#[derive(Default, Debug)]
pub(super) struct Census {
    /// Passes (loop op index, pass number) that left registers, flags, accumulators and DM equal.
    pub identity_passes: Vec<(usize, usize)>,
    /// Float add / subtract items: both lanes with a zero operand, one lane, no lane.
    pub addsub: [u32; 3],
    /// Float multiply items, same split.
    pub mul: [u32; 3],
}

fn is_zero(v: u64) -> bool {
    (v >> 31) & 0xFF == 0
}

/// Runs `prog` on `m` (loops with at least `min_passes` passes are compared pass by pass).
fn run_probed(
    m: &mut Machine,
    prog: &Program,
    lo: usize,
    hi: usize,
    min_passes: i32,
    c: &mut Census,
) {
    let mut p = lo;
    while p < hi {
        let row = prog.ops[p];
        p += 1;
        match row[0] {
            ctl::JUMP => {
                if row[1] == 1 || (row[1] == 2) != m.az[0] {
                    p = row[2] as usize;
                }
            }
            ctl::LOOP => {
                let body = row[2] as usize;
                for pass in 0..row[1] as usize {
                    let before = (m.d, m.an, m.az, m.mr, m.mem.clone());
                    run_probed(m, prog, p, p + body, min_passes, c);
                    let same = before.0 == m.d
                        && (before.1, before.2, before.3) == (m.an, m.az, m.mr)
                        && before.4 == m.mem;
                    if same && row[1] >= min_passes {
                        c.identity_passes.push((p, pass));
                    }
                }
                m.lcntr = 0;
                p += body;
            }
            _ => {
                let op = row[0] - ALUK;
                if op == A::FMUL || op == A::FADD || op == A::FSUB {
                    let (x, y) = (row[3] as usize & 15, row[4] as usize & 15);
                    let lane = |l: usize| is_zero(m.d[16 * l + x]) || is_zero(m.d[16 * l + y]);
                    let n = usize::from(!(lane(0) && lane(1))) + usize::from(!(lane(0) || lane(1)));
                    let t = if op == A::FMUL {
                        &mut c.mul
                    } else {
                        &mut c.addsub
                    };
                    t[n] += 1;
                }
                m.run_item(&row);
            }
        }
    }
}

fn machine(image: &BassImage, keys: bool) -> Machine {
    let mut m = Machine::new();
    m.load_image(&image.csv);
    if keys {
        m.load_image(include_str!(
            "../../../../../docs/research/data/bass-fw-chord-delta.csv"
        ));
    }
    m.set_programs(Some(&image.gen), &image.proc);
    m
}

/// Generator census over `calls` calls after a warm-up.
fn census(image: &BassImage, keys: bool, calls: usize) -> Census {
    let mut m = machine(image, keys);
    for _ in 0..500 {
        m.step();
    }
    let prog = compile(&image.gen);
    let mut c = Census::default();
    for _ in 0..calls {
        (m.i[0], m.i[1]) = GEN_PRESET;
        run_probed(&mut m, &prog, 0, prog.ops.len(), 10, &mut c);
        m.call_main(PROC_PRESET);
    }
    c
}

#[test]
fn no_pass_of_the_long_generator_loops_is_an_identity_with_keys_or_without() {
    // VERIFIED (run, 40 calls each): a skipped pass would have to reproduce its register, flag
    // and DM effect, and every pass of the 22 and 10 pass loops changes one of them.
    // needs the user's OS update file (`NS4_WAVE_TABLES`); skipped with a note without it
    let Some(image) = real_image() else { return };
    for keys in [true, false] {
        let c = census(&image, keys, 40);
        assert!(
            c.identity_passes.is_empty(),
            "keys {keys}: identity passes {:?}",
            c.identity_passes
        );
    }
}

#[test]
fn float_adds_of_the_generator_rarely_have_a_zero_operand_but_multiplies_often_do() {
    let Some(image) = real_image() else { return };
    let c = census(&image, true, 40);
    let n = |t: [u32; 3]| f64::from(t.iter().sum::<u32>());
    // index 0: both lanes have a zero operand, 1: one lane, 2: none
    assert!(f64::from(c.addsub[2]) > 0.75 * n(c.addsub), "{c:?}");
    assert!(f64::from(c.mul[0]) > 0.3 * n(c.mul), "{c:?}");
}
