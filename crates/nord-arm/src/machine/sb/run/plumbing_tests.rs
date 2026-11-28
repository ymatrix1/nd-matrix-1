//! OWNER: W2-DRV (tests). The driver's plumbing: the flipped re-layout after a hot guard, a
//! compiled trace run through a backend (including its `EXIT_SLOW` and `EXIT_STEPPED` results),
//! the tier off, and volatile and failed pcs.

use super::cycle_tests::{compare, counting, machine, state, CODE, EQ};
use super::run_fast;
use crate::cpu::Cpu;
use crate::machine::sb::backend::{Backend, RunOut, Tier2Backend, EXIT_SLOW, EXIT_STEPPED};
use crate::machine::sb::testgen::asm::{self, AL};
use crate::machine::sb::uop::k;
use crate::machine::sb::{SbConfig, SbState};
use crate::machine::{Machine, SysBus};

#[test]
fn a_hot_guard_triggers_one_flipped_rebuild_and_the_result_stays_exact() {
    // 00 mov r1,#0 ; 04 L: add r0,r0,#1 ; 08 cmp r1,#0 ; 0c beq 14 ; 10 add r2,r2,#1 ;
    // 14 add r3,r3,#1 ; 18 b L        (the forward beq is always taken: a hot guard exit)
    let words = [
        asm::mov_imm(1, 0),
        asm::add_imm(false, 0, 0, 1),
        asm::cmp_imm(1, 0),
        asm::branch(EQ, false, CODE + 12, CODE + 20),
        asm::add_imm(false, 2, 2, 1),
        asm::add_imm(false, 3, 3, 1),
        asm::branch(AL, false, CODE + 24, CODE + 4),
    ];
    let (mut a, mut b) = (machine(&words), machine(&words));
    let mut sb = state(SbConfig::on());
    compare(&mut a, &mut b, &mut sb, 3000);
    assert_eq!(b.cpu.r[2], 0, "the fall-through side never ran");
    assert!(
        sb.stats.dropped >= 1,
        "the first layout was replaced: {:?}",
        sb.stats
    );
    let loops = sb
        .cache
        .iter_live()
        .any(|(_, t)| t.uops.iter().any(|u| u.kind == k::LOOP_BACK));
    assert!(
        loops,
        "flipped, the taken side stays in the trace and closes the loop"
    );
}

/// A backend that wraps the tier-2 executor and, per `mode`, answers `EXIT_SLOW` (ran nothing)
/// or steps one instruction itself (`EXIT_STEPPED`), counting what it did.
struct Odd {
    inner: Tier2Backend,
    mode: u8,
    asked: u64,
}

impl Backend for Odd {
    fn compile(&mut self, sb: &crate::machine::sb::Sb) -> Option<u32> {
        self.inner.compile(sb)
    }
    fn release(&mut self, id: u32) {
        self.inner.release(id);
    }
    fn name(&self) -> &'static str {
        "odd"
    }
    fn run(&mut self, id: u32, cpu: &mut Cpu, bus: &mut SysBus, left: &mut u64) -> RunOut {
        self.asked += 1;
        match self.mode {
            0 => RunOut {
                exit: EXIT_SLOW,
                target: 0,
            },
            1 if self.asked.is_multiple_of(2) => {
                bus.pc = cpu.r[15];
                let _ = cpu.step(bus);
                *left -= 1;
                RunOut {
                    exit: EXIT_STEPPED,
                    target: cpu.r[15],
                }
            }
            _ => self.inner.run(id, cpu, bus, left),
        }
    }
}

#[test]
fn a_compiled_trace_runs_through_the_backend_and_its_odd_results_are_handled() {
    let words = counting(120);
    for mode in [2u8, 0, 1] {
        let (mut a, mut b) = (machine(&words), machine(&words));
        let mut sb = state(SbConfig {
            native: true,
            ..SbConfig::on()
        });
        sb.backend = Some(Box::new(Odd {
            inner: Tier2Backend::default(),
            mode,
            asked: 0,
        }));
        compare(&mut a, &mut b, &mut sb, 400);
        assert!(
            sb.stats.native_enters > 0,
            "mode {mode}: the backend was asked"
        );
        assert_eq!(
            sb.stats.native_slow_exits > 0,
            mode != 2,
            "mode {mode}: {:?}",
            sb.stats
        );
        if mode == 2 {
            assert!(sb.stats.guest_in_traces > 300);
        }
    }
}

#[test]
fn the_tier_off_is_the_predecode_loop_and_builds_nothing() {
    let words = counting(50);
    let (mut a, mut b) = (machine(&words), machine(&words));
    let mut sb = SbState::default();
    compare(&mut a, &mut b, &mut sb, 300);
    assert_eq!(
        (sb.stats.built, sb.stats.enters, sb.cache.live_count()),
        (0, 0, 0)
    );
}

#[test]
fn a_volatile_page_is_never_traced_and_a_failed_pc_is_not_retried() {
    let words = counting(50);
    let mut m = machine(&words);
    let mut sb = state(SbConfig::on());
    sb.cache.mark_volatile(CODE >> 12);
    run_fast(&mut m, &mut sb, 100, 0, u32::MAX);
    assert_eq!((sb.stats.built, sb.stats.build_failed), (0, 0));
    // an unmapped hot pc fails once and is then left to tier 1 (which maps and decodes it)
    let mut m = Machine::new();
    m.cpu.r[15] = 0x5000_0000;
    let mut sb = state(SbConfig::on());
    run_fast(&mut m, &mut sb, 8, 0, u32::MAX);
    assert_eq!(sb.stats.build_failed, 1);
    assert!(sb.hot.failed(0x5000_0000));
}
