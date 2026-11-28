//! OWNER: W1-CACHE. Tests of the stale path on a real `Mem`: stores, hooks and literals reach
//! `SbState::on_stale` the way `Mem::stale_code` delivers them. Each scenario is a plain
//! function so `mutants_are_killed` can run it again with one mutation site switched on.

use super::{is_active, SbState};
use crate::machine::sb::cache::tiny_trace;
use crate::machine::sb::uop::SbId;
use crate::machine::sb::{mutants, SbConfig};
use crate::mem::{Mem, HOOK_C, HOOK_X};

/// A state and a memory, driven the way `run::drain_stale` drives them.
pub(super) struct World {
    pub(super) st: SbState,
    pub(super) mem: Mem,
}

impl World {
    pub(super) fn new() -> Self {
        Self {
            st: SbState {
                cfg: SbConfig::on(),
                ..SbState::default()
            },
            mem: Mem::new(),
        }
    }

    /// A trace entered at `entry` that read the words at `addrs` as memory holds them now; the
    /// pages are flagged as code, as `run_fast` does after a build.
    pub(super) fn trace(&mut self, entry: u32, addrs: &[u32]) -> SbId {
        let words: Vec<(u32, u32)> = addrs.iter().map(|&a| (a, self.mem.read32(a))).collect();
        for &a in addrs {
            self.mem.mark_code(a);
        }
        self.st.cache.insert(tiny_trace(entry, &words))
    }

    /// Hand every queued page to `on_stale`, then empty the queue.
    pub(super) fn drain(&mut self) {
        let queue: Vec<u32> = self.mem.stale_code.drain(..).collect();
        for page in queue {
            self.st.on_stale(page, &mut self.mem);
        }
    }

    pub(super) fn live(&self, ids: &[SbId]) -> Vec<bool> {
        ids.iter().map(|&id| self.st.cache.is_live(id)).collect()
    }
}

fn data_store_on_a_code_page_keeps_unchanged_traces() {
    let mut w = World::new();
    w.mem.write32(0x1000, 0xA);
    w.mem.write32(0x1800, 0xB);
    let (a, b) = (w.trace(0x1000, &[0x1000]), w.trace(0x1800, &[0x1800]));
    w.mem.write32(0x1FF0, 7); // data on the code page, read by no trace
    assert_eq!(w.mem.stale_code, vec![1]);
    w.drain();
    assert_eq!(w.live(&[a, b]), [true, true]);
    assert_eq!((w.st.stats.dropped, w.st.stats.revalidated), (0, 2));
    assert_ne!(
        w.mem.flags(0x1000) & HOOK_C,
        0,
        "the page is flagged as code again"
    );
    w.mem.write32(0x1FF4, 8);
    assert_eq!(w.mem.stale_code, vec![1], "so the next store is seen too");
}

fn a_changed_word_drops_exactly_the_traces_that_read_it() {
    let mut w = World::new();
    for (addr, word) in [(0x1000, 0xA), (0x1004, 0xB), (0x1800, 0xC)] {
        w.mem.write32(addr, word);
    }
    let a = w.trace(0x1000, &[0x1000, 0x1004]);
    let b = w.trace(0x1800, &[0x1800]);
    let c = w.trace(0x1400, &[0x1400, 0x1004]);
    for pc in [0x1000, 0x1800, 0x1400] {
        w.st.hot.bump(pc);
        w.st.hot.bump(pc);
    }
    w.mem.write32(0x1004, 0xBB);
    w.drain();
    assert_eq!(w.live(&[a, b, c]), [false, true, false]);
    assert_eq!((w.st.stats.dropped, w.st.stats.revalidated), (2, 1));
    assert_eq!(
        (
            w.st.cache.lookup(0x1000),
            w.st.cache.lookup(0x1800),
            w.st.cache.lookup(0x1400)
        ),
        (None, Some(b), None)
    );
    assert_eq!(
        (w.st.hot.bump(0x1000), w.st.hot.bump(0x1800)),
        (1, 3),
        "a dropped trace's entry count restarts, a kept one's does not"
    );
    assert_ne!(
        w.mem.flags(0x1000) & HOOK_C,
        0,
        "the survivor keeps its page flagged"
    );
}

fn a_changed_folded_literal_drops_its_trace_and_an_unchanged_one_does_not() {
    let mut w = World::new();
    for (addr, word) in [(0x2000, 0x11), (0x2010, 0x33), (0x5008, 0x22)] {
        w.mem.write32(addr, word);
    }
    let t = w.trace(0x2000, &[0x2000, 0x5008]); // the literal sits on page 5
    let u = w.trace(0x2010, &[0x2010]);
    w.mem.write32(0x5008, 0x23);
    assert_eq!(
        w.mem.stale_code,
        vec![5],
        "only the literal's page went stale"
    );
    w.drain();
    assert_eq!(w.live(&[t, u]), [false, true]);
    assert_ne!(w.mem.flags(0x2000) & HOOK_C, 0, "page 2 was never touched");
    let t2 = w.trace(0x2000, &[0x2000, 0x5008]);
    w.mem.write32(0x5008, 0x23); // stored, but the same value
    w.drain();
    assert_eq!(w.live(&[t2]), [true]);
    assert_eq!(
        w.st.stats.revalidated, 1,
        "only t2: the first event left no trace on page 5"
    );
}

fn a_hook_on_the_page_drops_every_trace_and_is_no_code_change() {
    let mut w = World::new();
    w.mem.write32(0x1000, 1);
    w.mem.write32(0x1800, 2);
    w.mem.write32(0x2000, 3);
    let (a, b, c) = (
        w.trace(0x1000, &[0x1000]),
        w.trace(0x1800, &[0x1800]),
        w.trace(0x2000, &[0x2000]),
    );
    w.mem.add_flag(0x1000, 0x1FFF, HOOK_X);
    assert_eq!(w.mem.stale_code, vec![1]);
    w.drain();
    assert_eq!(
        w.live(&[a, b, c]),
        [false, false, true],
        "the words were intact all the same"
    );
    assert_eq!(
        w.mem.flags(0x1000) & (HOOK_C | HOOK_X),
        HOOK_X,
        "not flagged as code again"
    );
    assert!(!w.st.cache.volatile(1));
    assert_eq!(
        w.st.cache.note_change(1),
        1,
        "a hook is not a change of the code"
    );
}

fn a_page_whose_code_changed_five_times_turns_volatile() {
    let mut w = World::new();
    assert_eq!(w.st.cfg.volatile_after, 4);
    w.mem.write32(0x1000, 0);
    let first = w.trace(0x1000, &[0x1000]);
    for _ in 0..10 {
        w.mem.write32(0x1FF0, 1); // stores that change no word a trace read do not count
        w.drain();
    }
    assert!(w.st.cache.is_live(first) && !w.st.cache.volatile(1));
    for n in 1..=5u32 {
        let t = w.trace(0x1000, &[0x1000]);
        w.mem.write32(0x1000, n);
        w.drain();
        assert!(!w.st.cache.is_live(t), "change {n} drops the trace");
        assert_eq!(w.st.cache.volatile(1), n == 5, "after change {n}");
    }
}

fn a_volatile_page_keeps_no_trace_even_an_unchanged_one() {
    let mut w = World::new();
    w.mem.write32(0x1000, 1);
    let t = w.trace(0x1000, &[0x1000]);
    w.st.cache.mark_volatile(1);
    w.mem.write32(0x1FF0, 5);
    w.drain();
    assert_eq!(w.live(&[t]), [false]);
    assert_eq!(
        w.mem.flags(0x1000) & HOOK_C,
        0,
        "and the page is left alone"
    );
    assert_eq!(w.st.stats.revalidated, 0);
}

/// 20 traces on one page, every second one invalidated: the walk from the back must visit all.
fn dropping_many_traces_of_one_page_in_one_event_leaves_exactly_the_others() {
    let mut w = World::new();
    for i in 0..20u32 {
        w.mem.write32(0x1000 + 4 * i, i);
    }
    let ids: Vec<SbId> = (0..20u32)
        .map(|i| w.trace(0x1000 + 4 * i, &[0x1000 + 4 * i]))
        .collect();
    for i in (0..20u32).step_by(2) {
        w.mem.write32(0x1000 + 4 * i, 100 + i);
    }
    w.drain();
    let kept: Vec<SbId> = ids.iter().copied().filter(|id| id % 2 == 1).collect();
    assert_eq!(w.st.cache.covering(1), kept.as_slice());
    assert_eq!((w.st.stats.dropped, w.st.stats.revalidated), (10, 10));
    assert_eq!(w.st.cache.live_count(), 10);
}

#[test]
fn the_scenarios_hold_without_a_mutation() {
    data_store_on_a_code_page_keeps_unchanged_traces();
    a_changed_word_drops_exactly_the_traces_that_read_it();
    a_changed_folded_literal_drops_its_trace_and_an_unchanged_one_does_not();
    a_hook_on_the_page_drops_every_trace_and_is_no_code_change();
    a_page_whose_code_changed_five_times_turns_volatile();
    a_volatile_page_keeps_no_trace_even_an_unchanged_one();
    dropping_many_traces_of_one_page_in_one_event_leaves_exactly_the_others();
}

#[test]
fn mutants_are_killed() {
    let cases: [(u32, &str, fn()); 6] = [
        (
            650,
            "revalidation accepts a changed word",
            a_changed_word_drops_exactly_the_traces_that_read_it,
        ),
        (
            651,
            "kept page not flagged again",
            data_store_on_a_code_page_keeps_unchanged_traces,
        ),
        (
            652,
            "hooked page keeps traces",
            a_hook_on_the_page_drops_every_trace_and_is_no_code_change,
        ),
        (
            653,
            "volatile one change early",
            a_page_whose_code_changed_five_times_turns_volatile,
        ),
        (
            654,
            "dropped trace keeps its count",
            a_changed_word_drops_exactly_the_traces_that_read_it,
        ),
        (
            655,
            "killed trace stays in the entry table",
            a_changed_word_drops_exactly_the_traces_that_read_it,
        ),
    ];
    assert!(!is_active(650), "no mutation is active outside `with`");
    for (id, name, scenario) in cases {
        let killed = std::panic::catch_unwind(|| mutants::with(id, scenario)).is_err();
        assert!(killed, "mutant {id} ({name}) survived");
    }
}
