//! OWNER: W2-NMEM. Block-transfer and shifted-offset comparison suites of the native memory
//! tests (`cfg(test)`), kept apart from `tests.rs` so each file stays small.

use super::harness::oracle::{run_case, Case, Tally};
use super::harness::*;
use super::tests::one;
use crate::machine::sb::exec_mem::{F_P, F_U, F_WB};
use crate::machine::sb::native::rt::Jit;
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::{k, Uop};

/// Every `(U, P, W)` of `ldm`/`stm`, the base in and out of the list, as the lowest register or
/// not, lists that end on the last word of a page and lists that pass it.
pub(super) fn block_suite(jit: &mut Jit, rng: &mut Rng, cases: u32) -> Result<Tally, String> {
    let mut t = Tally::default();
    for kind in [k::LDM, k::STM] {
        for flags in 0..8u8 {
            for (rn, list) in [
                (1u8, 0b0000_1110u32),
                (2, 0b0111_1110),
                (4, 0b0001_0000),
                (0, 0b0111_1111_1111_1110),
                (6, 0b0110_0010),
            ] {
                let mut u = Uop::new(kind, 0x1000);
                u.rn = rn;
                u.imm = list;
                u.sh = list.count_ones() as u8;
                u.rs = flags;
                let sb = one(u);
                let native = compile_test(jit, &sb).ok_or("did not compile")?;
                for c in 0..cases {
                    let mut cpu = random_state(rng, true);
                    // bases around the end of a page, in or out of the fast pages
                    let page = [RAM0, RAM1, RAM2, HOOKED_R, CODE, UNMAPPED][c as usize % 6];
                    let skew = rng.below(4) * (c & 1);
                    cpu.r[rn as usize] = page_addr(page) + 0x1000 - 4 * rng.below(20) + skew;
                    run_case(jit, &native, &sb, &Case::new(cpu, c), &mut t)
                        .map_err(|e| format!("{kind} flags {flags} rn {rn} list {list:#b}: {e}"))?;
                }
            }
        }
    }
    Ok(t)
}

/// Register offsets of every shift type, including the encodings `LSR #0` and `ASR #0` (= 32),
/// with offsets that have the top bit set, and the absolute literal load at every alignment.
pub(super) fn shift_and_abs_suite(jit: &mut Jit, rng: &mut Rng) -> Result<Tally, String> {
    let mut t = Tally::default();
    let shifts = [
        (0u8, 0u8),
        (0, 5),
        (1, 0),
        (1, 7),
        (2, 0),
        (2, 31),
        (3, 1),
        (3, 31),
    ];
    for kind in [k::LDR_REG, k::STR_REG, k::LDRB_REG, k::STRB_REG] {
        for (ty, amount) in shifts {
            for flags in [F_U | F_P, F_P, F_U | F_P | F_WB, F_WB] {
                let mut u = Uop::new(kind, 0x1000);
                (u.rd, u.rn, u.rm, u.sh, u.rs) = (1, 0, 8, ty << 5 | amount, flags);
                let sb = one(u);
                let native = compile_test(jit, &sb).ok_or("did not compile")?;
                for (c, off) in [0xFFFF_FF00u32, 0x8000_0000, 0x40, 5]
                    .into_iter()
                    .enumerate()
                {
                    let mut cpu = random_state(rng, true);
                    cpu.r[0] = page_addr(RAM1) + 0x800;
                    cpu.r[8] = off;
                    run_case(jit, &native, &sb, &Case::new(cpu, c as u32), &mut t)
                        .map_err(|e| format!("{kind} shift {ty}/{amount} flags {flags}: {e}"))?;
                }
            }
        }
    }
    for page in [RAM0, RAM2, HOOKED_R, HOOKED_W, UNMAPPED] {
        for off in [0u32, 1, 2, 3, 0x10, 0xFFC, 0xFFD, 0xFFF] {
            let mut u = Uop::new(k::LDR_ABS, 0x1000);
            (u.rd, u.imm) = (2, page_addr(page) + off);
            let sb = one(u);
            let native = compile_test(jit, &sb).ok_or("did not compile")?;
            let case = Case::new(random_state(rng, true), off);
            run_case(jit, &native, &sb, &case, &mut t)
                .map_err(|e| format!("ldr_abs {:#x}: {e}", u.imm))?;
        }
    }
    Ok(t)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn both_suites_run_and_exercise_blocks_writebacks_and_slow_exits() {
        let Some(mut jit) = native_or_skip("both_suites_run") else {
            return;
        };
        let mut rng = Rng::new(0x5171);
        let t = block_suite(&mut jit, &mut rng, 6).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            t.blocks > 50 && t.writebacks > 20 && t.slow_exits > 10,
            "{t:?}"
        );
        let t = shift_and_abs_suite(&mut jit, &mut rng).unwrap_or_else(|e| panic!("{e}"));
        assert!(t.completed > 50 && t.slow_exits > 5, "{t:?}");
    }
}
