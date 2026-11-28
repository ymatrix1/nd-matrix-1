//! OWNER: W1-XALU. The `MUL` / `MLA` arm of the tier-2 executor.
//!
//! Spec (design note 3.1, WP W1-XALU "native `mul/mla`"): `rd = rm * rs (+ rn)` modulo 2^32, the
//! semantics of the `0 | 1` case of `Cpu::arm_mul`. With S the N and Z flags come from the
//! result and C and V are left alone (ARMv5). `rd`, `rn` (the accumulator), `rm` and `rs` are
//! registers `0..=14` (I2), every source is read before `rd` is written, so `rd == rm`,
//! `rd == rs` and `rn == rd` are all exact. The lowerer fills `u.rd` (instruction bits 19..16),
//! `u.rn` (15..12), `u.rs` (11..8), `u.rm` (3..0).
//!
//! Entry points: [`exec_mul`] is the const-generic arm (`ACC`, `S` known statically), and
//! [`exec_mul_with`] the same arm with `(acc, s)` as run-time flags, so a caller that learns the
//! pair from a kind id needs no const-generic dispatch of its own.
//!
//! Reachability: the frozen contract allocates no kind id for multiplies (`uop.rs` note, `k.rs`:
//! ids `108..=111` are reserved and `unreachable!` in an executor), so [`exec_dp`](super::exec_dp)
//! does not dispatch here and `k::family` maps no id to this arm; a guest `mul` is `GENERIC`
//! today. Making it reachable takes edits to files this package does not own: allocate the ids
//! (`uop/k.rs`: constants, `family`, `DEFINED`, `COUNT`), route multiplies out of
//! `LowerFamily::Generic` (`decode.rs::lower_family`), emit them in `lower_dp.rs`, and call
//! `exec_mul_with` from the `run_sb` arm (`exec.rs`). Until then `W2-NDP` has no kind to lower
//! for `mul`.
//!
//! Mutation sites 314, 315 and 316 (see `MUTANTS` in the parent module).

use super::{mutant, nz, put, reg, Regs};
use crate::machine::sb::uop::Uop;

/// Execute `MUL` (`ACC` false) or `MLA` (`ACC` true), setting N and Z when `S`.
#[inline(always)]
pub fn exec_mul<const ACC: bool, const S: bool>(r: &mut Regs, f: &mut u8, u: &Uop) {
    let product = reg(r, u.rm).wrapping_mul(reg(r, u.rs));
    let v = if ACC && !mutant(314) {
        product.wrapping_add(reg(r, u.rn))
    } else {
        product
    };
    put(r, u.rd, v);
    if S && !mutant(315) {
        *f = nz(v) | (*f & 3);
    }
}

/// [`exec_mul`] with `(acc, s)` chosen at run time: `acc` selects `MLA`, `s` the flag-setting form.
#[inline]
pub fn exec_mul_with(r: &mut Regs, f: &mut u8, u: &Uop, acc: bool, s: bool) {
    let (acc, s) = if mutant(316) { (s, acc) } else { (acc, s) };
    match (acc, s) {
        (false, false) => exec_mul::<false, false>(r, f, u),
        (true, false) => exec_mul::<true, false>(r, f, u),
        (false, true) => exec_mul::<false, true>(r, f, u),
        (true, true) => exec_mul::<true, true>(r, f, u),
    }
}

#[cfg(test)]
pub(super) use tests::suite;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Cpu;
    use crate::machine::sb::testkit::{random_cpu, Rng};
    use crate::machine::sb::uop::{k, pack_flags};

    /// ASSUMED (test input): 16-bit and 32-bit product boundaries; the oracle is `Cpu::arm_mul`.
    const EDGES: [u32; 9] = [
        0,
        1,
        2,
        0xFFFF,
        0x1_0000,
        0x7FFF_FFFF,
        0x8000_0000,
        0xFFFF_FFFF,
        0x5555_5555,
    ];

    /// What a run proved it did: cases, cases that changed `rd`, and which flags it saw set.
    #[derive(Default, Debug)]
    pub(crate) struct Seen {
        pub cases: u32,
        pub rd_changed: u32,
        pub flags_seen: u8,
    }

    fn insn(acc: bool, s: bool, u: &Uop) -> u32 {
        0xE000_0090
            | (acc as u32) << 21
            | (s as u32) << 20
            | (u.rd as u32) << 16
            | (u.rn as u32) << 12
            | (u.rs as u32) << 8
            | u.rm as u32
    }

    /// Run one case against `Cpu::arm_mul`; `Err` names the first difference.
    fn check<const ACC: bool, const S: bool>(
        cpu: &Cpu,
        u: &Uop,
        seen: &mut Seen,
    ) -> Result<(), String> {
        let mut want = cpu.clone();
        want.arm_mul(insn(ACC, S, u));
        let (mut r, mut f) = (cpu.r, pack_flags(cpu));
        exec_mul::<ACC, S>(&mut r, &mut f, u);
        if r != want.r || f != want.nzcv() as u8 {
            return Err(format!(
                "mul acc {ACC} s {S} {u:?}: got r {r:x?} f {f:04b}, want r {:x?} f {:04b}",
                want.r,
                want.nzcv()
            ));
        }
        let (mut r2, mut f2) = (cpu.r, pack_flags(cpu));
        exec_mul_with(&mut r2, &mut f2, u, ACC, S);
        if (r2, f2) != (r, f) {
            return Err(format!(
                "exec_mul_with({ACC}, {S}) differs from exec_mul {u:?}"
            ));
        }
        seen.cases += 1;
        seen.rd_changed += (r[u.rd as usize] != cpu.r[u.rd as usize]) as u32;
        seen.flags_seen |= f;
        Ok(())
    }

    fn all_four<F: FnMut(bool, bool, &Cpu, &Uop) -> Result<(), String>>(
        mut run: F,
        cpu: &Cpu,
        u: &Uop,
    ) -> Result<(), String> {
        run(false, false, cpu, u)?;
        run(true, false, cpu, u)?;
        run(false, true, cpu, u)?;
        run(true, true, cpu, u)
    }

    fn dispatch(acc: bool, s: bool, cpu: &Cpu, u: &Uop, seen: &mut Seen) -> Result<(), String> {
        match (acc, s) {
            (false, false) => check::<false, false>(cpu, u, seen),
            (true, false) => check::<true, false>(cpu, u, seen),
            (false, true) => check::<false, true>(cpu, u, seen),
            (true, true) => check::<true, true>(cpu, u, seen),
        }
    }

    /// Random registers (aliased on purpose a quarter of the time each) over biased states plus
    /// an edge-value sweep; `cases` random cases per `(acc, s)`. Used by the mutation test too.
    pub(crate) fn suite(seed: u64, cases: u32) -> Result<Seen, String> {
        let mut seen = Seen::default();
        let mut rng = Rng::new(seed);
        for _ in 0..cases {
            let mut x = [0u8; 4]; // rd, rn, rm, rs
            x.iter_mut().for_each(|v| *v = rng.below(15) as u8);
            match rng.below(6) {
                0 => x[2] = x[0], // rm == rd
                1 => x[3] = x[0], // rs == rd
                2 => x[1] = x[0], // rn == rd
                3 => x[3] = x[2], // rs == rm
                _ => {}
            }
            let u = Uop {
                rd: x[0],
                rn: x[1],
                rm: x[2],
                rs: x[3],
                ..Uop::new(k::GENERIC, 0x1000)
            };
            let cpu = random_cpu(&mut rng, 0x1008);
            all_four(|a, s, c, u| dispatch(a, s, c, u, &mut seen), &cpu, &u)?;
        }
        // edge values: every (rm, rs, rn) from EDGES, distinct registers, every flag nibble
        let u = Uop {
            rd: 0,
            rn: 1,
            rm: 2,
            rs: 3,
            ..Uop::new(k::GENERIC, 0x1000)
        };
        for (a, b, c) in EDGES.iter().flat_map(|a| {
            EDGES
                .iter()
                .flat_map(move |b| EDGES.iter().map(move |c| (*a, *b, *c)))
        }) {
            let mut cpu = Cpu::new();
            cpu.r[..4].copy_from_slice(&[0x1234_5678, c, a, b]);
            cpu.set_cpsr((rng.next() & 0xF000_0000) | 0x13);
            all_four(|acc, s, cp, u| dispatch(acc, s, cp, u, &mut seen), &cpu, &u)?;
        }
        Ok(seen)
    }

    #[test]
    fn mul_and_mla_equal_the_interpreter_and_the_run_was_not_vacuous() {
        let seen = suite(0x6D75_6C00, 2000).unwrap();
        assert_eq!(seen.cases, 4 * (2000 + 9 * 9 * 9));
        assert!(seen.rd_changed > seen.cases / 2, "{seen:?}");
        assert_eq!(
            seen.flags_seen, 0xF,
            "N, Z, C and V all seen set (C, V are untouched inputs)"
        );
    }

    #[test]
    fn s_sets_n_and_z_from_the_result_and_leaves_c_and_v() {
        let u = Uop {
            rd: 0,
            rn: 1,
            rm: 2,
            rs: 3,
            ..Uop::new(k::GENERIC, 0x1000)
        };
        let run = |acc: bool, regs: [u32; 4], f: u8| {
            let (mut r, mut f) = ([0u32; 16], f);
            r[..4].copy_from_slice(&regs);
            if acc {
                exec_mul::<true, true>(&mut r, &mut f, &u);
            } else {
                exec_mul::<false, true>(&mut r, &mut f, &u);
            }
            (r[0], f)
        };
        assert_eq!(run(false, [9, 0, 6, 7], 0b0011), (42, 0b0011));
        assert_eq!(
            run(false, [9, 0, 0x10000, 0x10000], 0b1011),
            (0, 0b0111),
            "wraps to zero: Z set, N cleared"
        );
        assert_eq!(run(false, [9, 0, 0xFFFF_FFFF, 2], 0), (0xFFFF_FFFE, 0b1000));
        assert_eq!(run(true, [9, 5, 6, 7], 0), (47, 0));
    }
}
