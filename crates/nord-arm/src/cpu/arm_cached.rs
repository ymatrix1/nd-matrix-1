//! Handlers of the predecoded ARM instructions ([`super::predecode`]). One function per decoded
//! class, with the opcode, S bit and operand form as const generics for data processing, so the
//! per-instruction `match` of the plain decoder folds away. Every handler computes exactly what
//! `Cpu::exec_arm_body` computes for the same word (differential test in `predecode`).

use super::predecode::TABLE_LEN;
use super::{Bus, Cpu, Fault};
use std::marker::PhantomData;

/// `(cpu, bus, insn)`; `r[15]` already holds `pc + 8` (so the instruction is at `r[15] - 8`) and
/// the condition already passed. Returns `true` when the instruction faulted, with the fault in
/// `cpu.fault`; this keeps the arguments in registers and the result out of memory.
pub type Handler<B> = fn(&mut Cpu, &mut B, u32) -> bool;

#[inline(always)]
fn done(c: &mut Cpu, r: Result<(), Fault>) -> bool {
    match r {
        Ok(()) => false,
        Err(f) => {
            c.fault = Some(f);
            true
        }
    }
}

#[inline(always)]
fn pc_of(c: &Cpu) -> u32 {
    c.r[15].wrapping_sub(8)
}

fn h_slow<B: Bus>(c: &mut Cpu, bus: &mut B, insn: u32) -> bool {
    let r = c.exec_arm_body(bus, pc_of(c), insn);
    done(c, r)
}

fn h_uncond<B: Bus>(c: &mut Cpu, bus: &mut B, insn: u32) -> bool {
    let r = c.exec_arm_uncond(bus, pc_of(c), insn);
    done(c, r)
}

#[inline(always)]
fn branch_off(insn: u32) -> u32 {
    (((insn & 0x00FF_FFFF) << 8) as i32 >> 6) as u32
}

fn h_b<B: Bus>(c: &mut Cpu, _: &mut B, insn: u32) -> bool {
    c.branch(c.r[15].wrapping_add(branch_off(insn)), false);
    false
}

fn h_bl<B: Bus>(c: &mut Cpu, _: &mut B, insn: u32) -> bool {
    c.r[14] = c.r[15].wrapping_sub(4); // pc + 4
    c.branch(c.r[15].wrapping_add(branch_off(insn)), false);
    false
}

fn h_bx<B: Bus>(c: &mut Cpu, _: &mut B, insn: u32) -> bool {
    let t = c.r[(insn & 0xF) as usize];
    c.branch(t, true);
    false
}

fn h_ldst<B: Bus, const L: bool, const BYTE: bool>(c: &mut Cpu, bus: &mut B, insn: u32) -> bool {
    c.ldst_exec(bus, insn, L, BYTE);
    false
}

fn h_ldm<B: Bus>(c: &mut Cpu, bus: &mut B, insn: u32) -> bool {
    c.arm_ldm(bus, insn);
    false
}

fn h_half<B: Bus>(c: &mut Cpu, bus: &mut B, insn: u32) -> bool {
    let r = c.arm_ldst_half(bus, pc_of(c), insn);
    done(c, r)
}

fn h_mul<B: Bus>(c: &mut Cpu, _: &mut B, insn: u32) -> bool {
    c.arm_mul(insn);
    false
}

fn h_dp<B: Bus, const OP: u32, const S: bool, const FORM: u32>(
    c: &mut Cpu,
    _: &mut B,
    insn: u32,
) -> bool {
    c.dp_exec(OP, S, FORM, insn);
    false
}

/// Handler table indexed by [`op`] ids.
pub struct Table<B>(PhantomData<B>);

impl<B: Bus + 'static> Table<B> {
    /// A `&'static` array, so a dispatch reads rodata instead of copying the constant.
    pub const ARM: &'static [Handler<B>; TABLE_LEN] = &[
        h_slow::<B>,               // op::UNDECODED (never dispatched)
        h_slow::<B>,               // op::SLOW
        h_uncond::<B>,             // op::UNCOND
        h_b::<B>,                  // op::B
        h_bl::<B>,                 // op::BL
        h_bx::<B>,                 // op::BX
        h_ldst::<B, true, false>,  // op::LDR
        h_ldst::<B, true, true>,   // op::LDRB
        h_ldst::<B, false, false>, // op::STR
        h_ldst::<B, false, true>,  // op::STRB
        h_ldm::<B>,                // op::LDM
        h_half::<B>,               // op::HALF
        h_mul::<B>,                // op::MUL
        h_slow::<B>,               // spare
        h_slow::<B>,               // spare
        h_slow::<B>,               // spare
        // form 0: opcode 0..15, S off then on
        h_dp::<B, 0, false, 0>,
        h_dp::<B, 0, true, 0>,
        h_dp::<B, 1, false, 0>,
        h_dp::<B, 1, true, 0>,
        h_dp::<B, 2, false, 0>,
        h_dp::<B, 2, true, 0>,
        h_dp::<B, 3, false, 0>,
        h_dp::<B, 3, true, 0>,
        h_dp::<B, 4, false, 0>,
        h_dp::<B, 4, true, 0>,
        h_dp::<B, 5, false, 0>,
        h_dp::<B, 5, true, 0>,
        h_dp::<B, 6, false, 0>,
        h_dp::<B, 6, true, 0>,
        h_dp::<B, 7, false, 0>,
        h_dp::<B, 7, true, 0>,
        h_dp::<B, 8, false, 0>,
        h_dp::<B, 8, true, 0>,
        h_dp::<B, 9, false, 0>,
        h_dp::<B, 9, true, 0>,
        h_dp::<B, 10, false, 0>,
        h_dp::<B, 10, true, 0>,
        h_dp::<B, 11, false, 0>,
        h_dp::<B, 11, true, 0>,
        h_dp::<B, 12, false, 0>,
        h_dp::<B, 12, true, 0>,
        h_dp::<B, 13, false, 0>,
        h_dp::<B, 13, true, 0>,
        h_dp::<B, 14, false, 0>,
        h_dp::<B, 14, true, 0>,
        h_dp::<B, 15, false, 0>,
        h_dp::<B, 15, true, 0>,
        // form 1: opcode 0..15, S off then on
        h_dp::<B, 0, false, 1>,
        h_dp::<B, 0, true, 1>,
        h_dp::<B, 1, false, 1>,
        h_dp::<B, 1, true, 1>,
        h_dp::<B, 2, false, 1>,
        h_dp::<B, 2, true, 1>,
        h_dp::<B, 3, false, 1>,
        h_dp::<B, 3, true, 1>,
        h_dp::<B, 4, false, 1>,
        h_dp::<B, 4, true, 1>,
        h_dp::<B, 5, false, 1>,
        h_dp::<B, 5, true, 1>,
        h_dp::<B, 6, false, 1>,
        h_dp::<B, 6, true, 1>,
        h_dp::<B, 7, false, 1>,
        h_dp::<B, 7, true, 1>,
        h_dp::<B, 8, false, 1>,
        h_dp::<B, 8, true, 1>,
        h_dp::<B, 9, false, 1>,
        h_dp::<B, 9, true, 1>,
        h_dp::<B, 10, false, 1>,
        h_dp::<B, 10, true, 1>,
        h_dp::<B, 11, false, 1>,
        h_dp::<B, 11, true, 1>,
        h_dp::<B, 12, false, 1>,
        h_dp::<B, 12, true, 1>,
        h_dp::<B, 13, false, 1>,
        h_dp::<B, 13, true, 1>,
        h_dp::<B, 14, false, 1>,
        h_dp::<B, 14, true, 1>,
        h_dp::<B, 15, false, 1>,
        h_dp::<B, 15, true, 1>,
        // form 2: opcode 0..15, S off then on
        h_dp::<B, 0, false, 2>,
        h_dp::<B, 0, true, 2>,
        h_dp::<B, 1, false, 2>,
        h_dp::<B, 1, true, 2>,
        h_dp::<B, 2, false, 2>,
        h_dp::<B, 2, true, 2>,
        h_dp::<B, 3, false, 2>,
        h_dp::<B, 3, true, 2>,
        h_dp::<B, 4, false, 2>,
        h_dp::<B, 4, true, 2>,
        h_dp::<B, 5, false, 2>,
        h_dp::<B, 5, true, 2>,
        h_dp::<B, 6, false, 2>,
        h_dp::<B, 6, true, 2>,
        h_dp::<B, 7, false, 2>,
        h_dp::<B, 7, true, 2>,
        h_dp::<B, 8, false, 2>,
        h_dp::<B, 8, true, 2>,
        h_dp::<B, 9, false, 2>,
        h_dp::<B, 9, true, 2>,
        h_dp::<B, 10, false, 2>,
        h_dp::<B, 10, true, 2>,
        h_dp::<B, 11, false, 2>,
        h_dp::<B, 11, true, 2>,
        h_dp::<B, 12, false, 2>,
        h_dp::<B, 12, true, 2>,
        h_dp::<B, 13, false, 2>,
        h_dp::<B, 13, true, 2>,
        h_dp::<B, 14, false, 2>,
        h_dp::<B, 14, true, 2>,
        h_dp::<B, 15, false, 2>,
        h_dp::<B, 15, true, 2>,
        // padding to a power of two so the dispatch index needs no bounds check
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
        h_slow::<B>,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::bus::FlatBus;

    #[test]
    fn table_has_one_handler_per_op_and_dp_ids_are_laid_out_form_major() {
        assert_eq!(Table::<FlatBus>::ARM.len(), TABLE_LEN);
        use crate::cpu::predecode::{op, N_OPS};
        assert_eq!(op::dp(0, 0, false), op::DP_BASE);
        assert_eq!(op::dp(2, 15, true), N_OPS as u8 - 1);
        assert!(TABLE_LEN.is_power_of_two() && TABLE_LEN >= N_OPS);
        assert_eq!(op::dp(1, 0, false), op::DP_BASE + 32);
    }
}
