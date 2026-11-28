//! OWNER: W2-NMEM. Test harness of the native memory and flow lowerers (`cfg(test)`): a fixed
//! guest world with one page of every kind the fast path distinguishes, biased random states, a
//! random trace generator, a test-only stand-in for the data-processing lowerer and the
//! comparison of native code against tier 2 (`oracle.rs`).
//!
//! # The world
//!
//! Nine pages from [`BASE`]: three plain RAM pages, `HOOK_R`, `HOOK_W`, a page of decoded code
//! (`HOOK_C`), a page that only carries `HOOK_X` (a data access there is fast), an unmapped page
//! and an unmapped page with `HOOK_R | HOOK_W`; one more mapped page sits behind them in the arena
//! (a deliberately wrong mutant that runs over a page end stays inside the arena).
//!
//! # The stand-in
//!
//! [`test_lower`] is the lowerer the tests build with: memory and control uops go to this
//! package's lowerers, and the few data-processing uops the traces need (`ADD_IMM`, `SUB_IMM`,
//! `MOV_IMM`, `CMP_IMM`, `CMP_REG`) to [`stub::dp_stub`], so the tests do not depend on W2-NDP.

pub(crate) mod compare;
pub(crate) mod oracle;
pub(crate) mod stub;

use super::super::available;
use super::super::codegen::build_function_with;
use super::super::emit::testing::{assert_fallback, verified};
use super::super::emit::{NCtx, Unsupported};
use super::super::lower_flow::lower_flow;
use super::super::rt::{Jit, Native};
use super::lower_mem;
use crate::cpu::Cpu;
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::{k, Exit, ExitKind, Sb, Uop};
use crate::mem::{Mem, HOOK_R, HOOK_W};
use stub::dp_stub;

/// First guest address of the test world.
pub(crate) const BASE: u32 = 0x9000_0000;
/// Roles of the pages of the world (`page_addr(i)`).
pub(crate) const RAM0: u32 = 0;
pub(crate) const RAM1: u32 = 1;
pub(crate) const RAM2: u32 = 2;
pub(crate) const HOOKED_R: u32 = 3;
pub(crate) const HOOKED_W: u32 = 4;
pub(crate) const CODE: u32 = 5;
pub(crate) const HOOKED_X: u32 = 6;
pub(crate) const UNMAPPED: u32 = 7;
pub(crate) const UNMAPPED_HOOKED: u32 = 8;
/// Pages a generated address may land in.
pub(crate) const PAGES: u32 = 9;

/// Address of page `i` of the world.
pub(crate) const fn page_addr(i: u32) -> u32 {
    BASE + (i << 12)
}

/// A deterministic copy of the world: the content of every mapped page comes from `seed`.
pub(crate) fn world(seed: u32) -> Mem {
    let mut m = Mem::new();
    let mut rng = Rng::new(u64::from(seed) | 0x100);
    for page in [
        RAM0,
        RAM1,
        RAM2,
        HOOKED_R,
        HOOKED_W,
        CODE,
        HOOKED_X,
        PAGES + 1,
    ] {
        for w in 0..1024 {
            m.write32(page_addr(page) + 4 * w, rng.next());
        }
    }
    m.add_flag(page_addr(HOOKED_R), page_addr(HOOKED_R) + 0xFFF, HOOK_R);
    m.add_flag(page_addr(HOOKED_W), page_addr(HOOKED_W) + 0xFFF, HOOK_W);
    m.mark_code(page_addr(CODE));
    m.add_flag(
        page_addr(HOOKED_X),
        page_addr(HOOKED_X) + 0xFFF,
        crate::mem::HOOK_X,
    );
    m.add_flag(
        page_addr(UNMAPPED_HOOKED),
        page_addr(UNMAPPED_HOOKED) + 0xFFF,
        HOOK_R | HOOK_W,
    );
    m
}

/// A guest address in one of the pages, biased to the page ends and to unaligned offsets.
pub(crate) fn addr_value(rng: &mut Rng) -> u32 {
    let page = rng.below(PAGES);
    let off = match rng.below(8) {
        0 => 0,
        1 => 0xFFC,
        2 => 0xFF8,
        3 => 0xFFD + rng.below(3),
        4 => rng.below(0x1000),
        5 => rng.below(0x400) * 4,
        6 => 0xFF0 + rng.below(16),
        _ => 0x1000 - 4 * rng.range(1, 16) + rng.below(4),
    };
    page_addr(page) + off
}

/// A small register value, positive or negative.
fn small_value(rng: &mut Rng) -> u32 {
    let v = rng.below(48);
    if rng.chance(4) {
        v.wrapping_neg()
    } else {
        v
    }
}

/// A random CPU: `r0..=r7` address-like, `r8..=r14` small (offsets) when `structured`, every
/// register a mix of address, small and biased value otherwise; random flags; `insns` random.
pub(crate) fn random_state(rng: &mut Rng, structured: bool) -> Cpu {
    let mut cpu = Cpu::new();
    for (i, reg) in cpu.r.iter_mut().enumerate().take(15) {
        *reg = if structured {
            if i < 8 {
                addr_value(rng)
            } else {
                small_value(rng)
            }
        } else {
            match rng.below(20) {
                0..=10 => addr_value(rng),
                11..=16 => small_value(rng),
                _ => rng.biased(),
            }
        };
    }
    cpu.r[15] = 0x1000;
    let nzcv = rng.below(16);
    cpu.n = nzcv & 8 != 0;
    cpu.z = nzcv & 4 != 0;
    cpu.c = nzcv & 2 != 0;
    cpu.v = nzcv & 1 != 0;
    cpu.insns = u64::from(rng.next() >> 8);
    cpu
}

/// A random memory uop at `pc` (kind from the whole family, every flag combination, `rn == rd`,
/// register offsets with every shift type, lists of any size). `structured` keeps `rn` in
/// `r0..=r7` and `rm` in `r8..=r14`, the registers [`random_state`] makes addresses and offsets.
pub(crate) fn random_mem_uop(rng: &mut Rng, pc: u32, structured: bool) -> Uop {
    let kind = rng.pick(&MEM_KINDS);
    random_mem_uop_of(rng, kind, pc, structured)
}

/// Every kind of the memory family.
pub(crate) const MEM_KINDS: [u8; 17] = [
    k::LDR_IMM,
    k::LDRB_IMM,
    k::STR_IMM,
    k::STRB_IMM,
    k::LDR_REG,
    k::LDRB_REG,
    k::STR_REG,
    k::STRB_REG,
    k::LDRH,
    k::LDRSH,
    k::LDRSB,
    k::STRH,
    k::LDRD,
    k::STRD,
    k::LDM,
    k::STM,
    k::LDR_ABS,
];

/// [`random_mem_uop`] of one given `kind`.
pub(crate) fn random_mem_uop_of(rng: &mut Rng, kind: u8, pc: u32, structured: bool) -> Uop {
    use crate::machine::sb::exec_mem::{F_P, F_REG, F_U, F_WB};
    let mut u = Uop::new(kind, pc);
    let pair = matches!(u.kind, k::LDRD | k::STRD);
    u.rd = rng.below(if pair { 14 } else { 15 }) as u8;
    u.rn = if structured {
        rng.below(8) as u8
    } else {
        rng.below(15) as u8
    };
    if rng.chance(5) {
        u.rn = u.rd.min(14);
    }
    u.rm = if structured {
        8 + rng.below(7) as u8
    } else {
        rng.below(15) as u8
    };
    let (up, pre) = (rng.flag(), rng.flag());
    let wb = !pre || rng.flag();
    u.rs = (u8::from(up) * F_U) | (u8::from(pre) * F_P) | (u8::from(wb) * F_WB);
    match u.kind {
        k::LDR_IMM | k::LDRB_IMM | k::STR_IMM | k::STRB_IMM => {
            u.imm = if rng.chance(3) {
                rng.below(0x1000)
            } else {
                rng.below(16)
            };
        }
        k::LDR_REG | k::LDRB_REG | k::STR_REG | k::STRB_REG => {
            let ty = rng.below(4) as u8;
            let amount = if ty == 3 {
                1 + rng.below(31)
            } else {
                rng.below(32)
            } as u8;
            u.sh = ty << 5 | amount;
        }
        k::LDRH | k::LDRSH | k::LDRSB | k::STRH | k::LDRD | k::STRD => {
            u.imm = rng.below(256);
            if rng.flag() {
                u.rs |= F_REG;
            }
        }
        k::LDM | k::STM => {
            let wb = rng.flag();
            u.rs = (u.rs & !F_WB) | (u8::from(wb) * F_WB);
            let mut list = 0;
            while list == 0 {
                list = rng.next() & 0x7FFF;
                if rng.flag() {
                    list &= rng.next() & 0x7FFF;
                }
            }
            u.imm = list;
            u.sh = list.count_ones() as u8;
        }
        _ => {
            u.imm = addr_value(rng) & !(if rng.chance(3) { 0 } else { 3 });
            u.rn = 0;
        }
    }
    u
}

/// A generated trace and the equalities its `GUARD_REG_EQ` uops want the state to satisfy.
pub(crate) struct Built {
    pub sb: Sb,
    /// `(register, value)` of every `GUARD_REG_EQ`: a case sets the register to the value about
    /// half of the time, so both outcomes of the guard are compared.
    pub eq_guards: Vec<(u8, u32)>,
}

/// `ADD_IMM rd, rn, #imm` (the stand-in lowerer knows it).
pub(crate) fn add_imm(rd: u8, rn: u8, imm: u32, pc: u32) -> Uop {
    let mut u = Uop::new(k::ADD_IMM, pc);
    (u.rd, u.rn, u.imm) = (rd, rn, imm);
    u
}

/// A random straight-line trace of memory, guard and `PRED` uops ending in one `EXIT`.
pub(crate) fn random_trace(rng: &mut Rng, structured: bool) -> Built {
    let mut uops: Vec<Uop> = Vec::new();
    let mut exits = vec![Exit::new(ExitKind::Stale, 0, 0x1000, 0x1004)];
    let mut eq_guards = Vec::new();
    let (mut pc, mut n) = (0x1000u32, 0u16);
    for _ in 0..1 + rng.below(7) {
        match rng.below(12) {
            0..=6 => {
                uops.push(random_mem_uop(rng, pc, structured));
                (pc, n) = (pc + 4, n + 1);
            }
            7 => {
                let len = 1 + rng.below(3) as usize;
                let mut p = Uop::new(k::PRED, pc);
                (p.cond, p.imm, p.n) = (rng.below(16) as u8, len as u32, 0);
                uops.push(p);
                for _ in 0..len {
                    if rng.chance(4) {
                        uops.push(add_imm(rng.below(15) as u8, rng.below(15) as u8, 1, pc));
                    } else {
                        uops.push(random_mem_uop(rng, pc, structured));
                    }
                    (pc, n) = (pc + 4, n + 1);
                }
            }
            8 | 9 => {
                let mut g = Uop::new(k::GUARD, pc);
                (g.cond, g.rs, g.n) = (rng.below(16) as u8, exits.len() as u8, 0);
                exits.push(Exit::new(ExitKind::Direct, n + 1, pc, pc + 0x40));
                uops.push(g);
                (pc, n) = (pc + 4, n + 1);
            }
            10 => {
                let mut g = Uop::new(k::GUARD_REG_EQ, pc);
                let (rn, imm) = (rng.below(15) as u8, addr_value(rng));
                (g.rn, g.imm, g.rs, g.n) = (rn, imm, exits.len() as u8, 0);
                exits.push(Exit::new(ExitKind::Direct, n + 1, pc, imm));
                eq_guards.push((rn, imm));
                uops.push(g);
                (pc, n) = (pc + 4, n + 1);
            }
            _ => {
                uops.push(add_imm(
                    rng.below(15) as u8,
                    rng.below(15) as u8,
                    rng.below(64),
                    pc,
                ));
                (pc, n) = (pc + 4, n + 1);
            }
        }
    }
    let last = exits.len();
    exits.push(Exit::new(ExitKind::Direct, n, pc - 4, pc));
    uops.push(Uop::exit(last as u8, pc - 4));
    let sb = Sb::from_parts(0x1000, n + 1, uops, exits, vec![(0x1000, 0)]).expect("a small trace");
    Built { sb, eq_guards }
}

/// The lowerer of the tests: this package's lowerers plus [`stub::dp_stub`].
pub(crate) fn test_lower(cx: &mut NCtx<'_>, u: &Uop) -> Result<(), Unsupported> {
    match k::family(u.kind) {
        k::Family::Dp => dp_stub(cx, u),
        k::Family::Mem => lower_mem(cx, u),
        k::Family::Flow | k::Family::Fused => lower_flow(cx, u),
        k::Family::Invalid => Err(Unsupported),
    }
}

/// Build the function of `sb` with [`test_lower`], verify the IR and compile it. `None` when the
/// trace does not end in a terminated block or the module refuses it.
pub(crate) fn compile_test(jit: &mut Jit, sb: &Sb) -> Option<Native> {
    let func = build_function_with(sb, jit.call_conv(), jit.ptr_type(), test_lower)?;
    jit.define_native(verified(func))
}

/// The module of a native test, or `None` (after printing why and asserting the fallback) where
/// native code is unavailable.
pub(crate) fn native_or_skip(test: &str) -> Option<Jit> {
    if available() {
        return Jit::new();
    }
    println!("skipped {test}: native code is unavailable on this host (the fallback is asserted)");
    assert_fallback();
    None
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn the_world_has_the_hook_pages_and_the_generators_stay_in_it() {
        let mut m = world(7);
        assert_eq!(m.read32(page_addr(RAM1)), world(7).read32(page_addr(RAM1)));
        assert_ne!(
            world(7).read32(page_addr(RAM1)),
            world(8).read32(page_addr(RAM1))
        );
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            let a = addr_value(&mut rng);
            assert!((BASE..page_addr(PAGES)).contains(&a), "{a:#x}");
        }
        let cpu = random_state(&mut rng, true);
        assert_eq!(cpu.r[15], 0x1000);
        for kind in MEM_KINDS {
            assert_eq!(random_mem_uop_of(&mut rng, kind, 0x1000, true).kind, kind);
        }
    }
}
