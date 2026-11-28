//! OWNER: W1-VGEN (split out of `testgen.rs` to keep that file under the size rule; the I7 and
//! I6 handling was added by the W2-XCORE fix). The tier stack without a cache.
//!
//! [`drive_traces`] is a copy of the outer loop of `Machine::run` that builds a trace per pc and
//! runs it with `exec::run_sb`. It is a (cacheless) tier driver, so it obeys the driver's side of
//! the contract of design note 4.1, which `run_sb` relies on:
//!
//! * I7: a trace is entered only when `left >= n_guest` and none of its pages holds `until` or
//!   `stop` ([`enterable`]); otherwise the pc takes one step of `Machine::run`, which honours the
//!   exact budget and the stop addresses.
//! * I6 and the stale drain: every page of a trace is marked as code (`Mem::mark_code`) before the
//!   trace runs, so a store into a trace's own page makes `Mem::stale_code` non-empty and the
//!   trace leaves through its `Stale` exit; the queue is drained (`Machine::drop_stale_code`)
//!   before every build and after every run, so the next trace is built from the new words.

use super::super::config::SbConfig;
use super::super::exec::run_sb;
use super::super::trace::{build, BuildOpts};
use super::super::uop::Sb;
use crate::machine::{Machine, Stop};
use crate::mem::HOOK_X;

/// I7: whether `sb` may be entered with `left` instructions of budget left and the stop addresses
/// `until` and `stop` (`u32::MAX` for none).
fn enterable(sb: &Sb, left: u64, until: u32, stop: u32) -> bool {
    u64::from(sb.n_guest) <= left && !sb.covers_page(until >> 12) && !sb.covers_page(stop >> 12)
}

/// `Machine::run` with the tier stack and no cache: at every pc build a trace (`trace::build`) and
/// run it (`exec::run_sb`) when I7 allows ([`enterable`]); Thumb, hooked, misaligned and unmapped
/// pcs, and the stretches too short for a whole trace, take one step of the real `Machine::run`.
/// Same contract and result as `Machine::run`.
pub(crate) fn drive_traces(
    m: &mut Machine,
    max: u64,
    until: u32,
    stop_pc: Option<u32>,
) -> (u64, Stop) {
    let start = m.cpu.insns;
    let limit = start + max;
    let stop = stop_pc.unwrap_or(u32::MAX);
    let opts = BuildOpts::from(&SbConfig::on());
    let reason = loop {
        if m.cpu.insns >= limit {
            break Stop::Budget;
        }
        let pc = m.cpu.r[15];
        if pc == until {
            break Stop::Until;
        }
        if pc == stop {
            break Stop::StopPc;
        }
        m.drop_stale_code();
        let left = limit - m.cpu.insns;
        let sb = if m.cpu.thumb || m.bus.mem.flags(pc) & HOOK_X != 0 {
            None
        } else {
            build(&mut m.bus.mem, pc, &opts).filter(|sb| enterable(sb, left, until, stop))
        };
        match sb {
            Some(sb) => {
                for page in &sb.pages[..sb.n_pages as usize] {
                    m.bus.mem.mark_code(page << 12);
                }
                let mut left = left;
                run_sb(&mut m.cpu, &mut m.bus, &sb, &mut left, false);
                m.drop_stale_code();
                if let Some(f) = m.cpu.fault.take() {
                    break Stop::Fault(f);
                }
            }
            None => match m.run(1, until, stop_pc) {
                (_, Stop::Budget) => {}
                (_, other) => break other,
            },
        }
    };
    (m.cpu.insns - start, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sb::testgen::{machine_with, CODE};

    /// `add r0,r0,#1` six times, then `b .`
    fn counting() -> Vec<u32> {
        let mut w = vec![0xE280_0001; 6];
        w.push(0xEAFF_FFFE);
        w
    }

    /// Run the same program through `Machine::run` and `drive_traces` and require equal results.
    fn same_as_run(words: &[u32], max: u64, until: u32, stop: Option<u32>) -> (Machine, Stop) {
        let (mut a, mut b) = (machine_with(words, 3, true), machine_with(words, 3, true));
        let ra = a.run(max, until, stop);
        let rb = drive_traces(&mut b, max, until, stop);
        assert_eq!(ra, rb, "result of {max} {until:#x} {stop:x?}");
        assert_eq!((a.cpu.r, a.cpu.insns), (b.cpu.r, b.cpu.insns));
        (b, rb.1)
    }

    #[test]
    fn a_budget_shorter_than_the_trace_is_honoured_to_the_instruction() {
        // the trace of the whole page is longer than any budget below: I7 sends the pc to the
        // interpreter for the last instructions instead of entering the trace
        for max in 1..=9 {
            let (m, why) = same_as_run(&counting(), max, 0, None);
            assert_eq!((why, m.cpu.insns), (Stop::Budget, max));
        }
    }

    #[test]
    fn until_and_stop_inside_the_page_of_a_trace_stop_exactly_there() {
        let (m, why) = same_as_run(&counting(), 50, CODE + 12, None);
        assert_eq!((why, m.cpu.insns, m.cpu.r[15]), (Stop::Until, 3, CODE + 12));
        let (m, why) = same_as_run(&counting(), 50, 0, Some(CODE + 8));
        assert_eq!((why, m.cpu.insns, m.cpu.r[15]), (Stop::StopPc, 2, CODE + 8));
    }

    #[test]
    fn a_store_into_the_trace_s_own_page_is_seen_by_the_next_instruction() {
        // str r3,[r2] rewrites the next word (mov r0,#2) to `mov r0,#7`: a trace that went on
        // with the old word would leave r0 = 2
        let words = [0xE582_3000, 0xE3A0_0002, 0xEAFF_FFFE];
        let (mut a, mut b) = (machine_with(&words, 1, true), machine_with(&words, 1, true));
        for m in [&mut a, &mut b] {
            (m.cpu.r[2], m.cpu.r[3]) = (CODE + 4, 0xE3A0_0007);
        }
        let ra = a.run(10, 0, None);
        let rb = drive_traces(&mut b, 10, 0, None);
        assert_eq!(ra, rb);
        assert_eq!((a.cpu.r, a.cpu.insns), (b.cpu.r, b.cpu.insns));
        assert_eq!(b.cpu.r[0], 7, "the rewritten instruction ran");
        assert!(b.bus.mem.stale_code.is_empty(), "the queue is drained");
    }
}
