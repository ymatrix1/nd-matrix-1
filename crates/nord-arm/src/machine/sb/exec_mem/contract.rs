//! Contract of the entry point: every memory kind has an arm that publishes the pc, a kind of
//! another family and the RRX register offset are refused, and the real `SysBus` runs the arms
//! (it sees the pc and reports a store into decoded code as stale).

use super::*;
use crate::machine::sb::testkit::LogBus;
use crate::machine::SysBus;

const PC: u32 = 0xC000_0100;

/// One arm run with every access checked against [`PC`].
fn run_uop(u: &Uop, init: &[(usize, u32)]) -> LogBus {
    let mut r = [0u32; 16];
    for &(i, v) in init {
        r[i] = v;
    }
    let mut bus = LogBus::new(0);
    exec_mem(&mut r, &mut bus, u);
    assert!(
        bus.log.iter().all(|e| e.3 == PC),
        "pc of every access: {:x?}",
        bus.log
    );
    bus
}

#[test]
fn every_memory_kind_has_an_arm_that_runs_and_publishes_the_pc() {
    let kinds: Vec<u8> = (0..=255u8)
        .filter(|kind| k::family(*kind) == k::Family::Mem)
        .collect();
    assert_eq!(kinds.len(), 17, "112..=128");
    for kind in kinds {
        let mut u = Uop::new(kind, PC);
        (u.rd, u.rn, u.rm, u.rs) = (1, 2, 3, F_U | F_P);
        if matches!(kind, k::LDM | k::STM) {
            (u.sh, u.imm) = (2, 0b11_0000);
        }
        let bus = run_uop(&u, &[(2, 0x2000)]);
        assert!(!bus.log.is_empty(), "kind {kind} accessed nothing");
    }
}

#[test]
#[should_panic(expected = "not a memory uop")]
fn a_kind_of_another_family_is_refused() {
    exec_mem(
        &mut [0; 16],
        &mut SysBus::new(),
        &Uop::new(k::MOV_IMM, 0x1000),
    );
}

#[test]
#[should_panic(expected = "RRX")]
fn a_rotate_by_zero_register_offset_is_refused_because_it_needs_the_carry() {
    let mut u = Uop::new(k::LDR_REG, PC);
    u.sh = 3 << 5;
    exec_mem(&mut [0; 16], &mut LogBus::new(0), &u);
}

#[test]
fn the_system_bus_sees_the_pc_and_a_store_into_decoded_code_makes_it_stale() {
    let mut bus = SysBus::new();
    bus.mem.write32(0x2004, 0xAABB_CCDD);
    bus.mem.write32(0x5000, 0);
    bus.mem.mark_code(0x5000);
    let mut ld = Uop::new(k::LDR_IMM, PC);
    (ld.rd, ld.rn, ld.rs, ld.imm) = (0, 1, F_U | F_P, 4);
    let mut r = [0u32; 16];
    r[1] = 0x2000;
    exec_mem(&mut r, &mut bus, &ld);
    assert_eq!((r[0], bus.pc, bus.stale()), (0xAABB_CCDD, PC, false));
    let mut st = Uop::new(k::STR_IMM, PC + 4);
    (st.rd, st.rn, st.rs, st.imm) = (0, 1, F_U | F_P, 4);
    r[1] = 0x5000;
    exec_mem(&mut r, &mut bus, &st);
    assert_eq!(
        (bus.pc, bus.stale(), bus.mem.read32(0x5004)),
        (PC + 4, true, 0xAABB_CCDD)
    );
}

/// The `debug_assert!`s that enforce I2 and the `sh` popcount: lowerers must never emit these.
#[cfg(debug_assertions)]
mod refused_in_debug_builds {
    use super::*;

    #[test]
    #[should_panic(expected = "I2")]
    fn a_load_into_r15_is_refused() {
        let mut u = Uop::new(k::LDR_IMM, PC);
        u.rd = 15;
        run_uop(&u, &[]);
    }

    #[test]
    #[should_panic(expected = "I2")]
    fn a_pc_base_is_refused() {
        let mut u = Uop::new(k::STRH, PC);
        u.rn = 15;
        run_uop(&u, &[]);
    }

    #[test]
    #[should_panic(expected = "I2")]
    fn r15_in_a_block_list_is_refused() {
        let mut u = Uop::new(k::STM, PC);
        (u.sh, u.imm) = (1, 1 << 15);
        run_uop(&u, &[]);
    }

    #[test]
    #[should_panic(expected = "popcount")]
    fn a_popcount_that_disagrees_with_the_list_is_refused() {
        let mut u = Uop::new(k::LDM, PC);
        (u.sh, u.imm) = (3, 0b11);
        run_uop(&u, &[]);
    }

    #[test]
    #[should_panic(expected = "I2")]
    fn a_register_pair_reaching_r15_is_refused() {
        let mut u = Uop::new(k::LDRD, PC);
        u.rd = 14;
        run_uop(&u, &[]);
    }
}
