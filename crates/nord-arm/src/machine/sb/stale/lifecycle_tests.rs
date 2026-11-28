//! OWNER: W1-CACHE. Tests of the drop paths of the stale code: links, unmapped words, failure
//! marks, backend release, the flush at the uop cap and `Clone`.

use super::revalidate_tests::World;
use crate::cpu::Cpu;
use crate::machine::sb::backend::{Backend, RunOut, EXIT_SLOW};
use crate::machine::sb::cache::tiny_trace;
use crate::machine::sb::uop::{Sb, NO_SB};
use crate::machine::SysBus;
use std::sync::{Arc, Mutex};

#[test]
fn links_to_a_dropped_trace_are_cleared_by_the_stale_path_and_by_drop_trace() {
    let mut w = World::new();
    w.mem.write32(0x1000, 1);
    w.mem.write32(0x2000, 2);
    w.mem.write32(0x3000, 3);
    let b = w.trace(0x2000, &[0x2000]);
    let d = w.trace(0x3000, &[0x3000]);
    let a = w.trace(0x1000, &[0x1000]);
    let x = &mut w.st.cache.get_mut(a).exits[0];
    (x.link, x.ic_id, x.ic_pc) = (b, [d, b], [0x3000, 0x2000]);
    w.mem.write32(0x2000, 22);
    w.drain();
    let x = &w.st.cache.get(a).exits[0];
    assert_eq!((x.link, x.ic_id, x.ic_pc), (NO_SB, [d, NO_SB], [0x3000, 0]));
    w.st.drop_trace(d);
    assert_eq!(w.st.cache.get(a).exits[0].ic_id, [NO_SB; 2]);
    w.st.drop_trace(d); // a dead id is ignored
    assert_eq!(w.st.stats.dropped, 2);
}

#[test]
fn a_word_on_an_unmapped_page_counts_as_changed_and_maps_nothing() {
    let mut w = World::new();
    w.mem.write32(0x1000, 1);
    w.mem.write32(0x2FF0, 0);
    let before = (w.mem.auto_mapped_count, w.mem.arena_bytes());
    let gone =
        w.st.cache
            .insert(tiny_trace(0x1000, &[(0x1000, 1), (0x9000, 5)]));
    let straddle =
        w.st.cache
            .insert(tiny_trace(0x2FF0, &[(0x2FF0, 0), (0x2FFE, 5)]));
    w.st.on_stale(9, &mut w.mem);
    w.st.on_stale(2, &mut w.mem); // the word at 0x2FFE ends on the unmapped page 3
    assert_eq!(w.live(&[gone, straddle]), [false, false]);
    assert_eq!((w.mem.auto_mapped_count, w.mem.arena_bytes()), before);
    assert!(!w.mem.is_mapped(0x9000) && !w.mem.is_mapped(0x3000));
}

#[test]
fn a_stale_page_lets_failed_builds_on_it_be_tried_again_and_no_other() {
    let mut w = World::new();
    w.st.hot.mark_failed(0x1004);
    w.st.hot.mark_failed(0x2004);
    w.st.on_stale(1, &mut w.mem);
    assert_eq!(
        (w.st.hot.failed(0x1004), w.st.hot.failed(0x2004)),
        (false, true)
    );
}

#[test]
fn two_pages_of_one_trace_going_stale_together_drop_it_once() {
    let mut w = World::new();
    w.mem.write32(0x1000, 1);
    w.mem.write32(0x2000, 2);
    let t = w.trace(0x1000, &[0x1000, 0x2000]);
    w.mem.write32(0x1000, 11);
    w.mem.write32(0x2000, 22);
    assert_eq!(w.mem.stale_code, vec![1, 2]);
    w.drain();
    assert_eq!((w.live(&[t]), w.st.stats.dropped), (vec![false], 1));
    assert_eq!(
        (w.st.cache.note_change(1), w.st.cache.note_change(2)),
        (2, 1)
    );
    assert!(w.st.cache.covering(1).is_empty() && w.st.cache.covering(2).is_empty());
}

/// A backend that only records the ids it is asked to release.
struct Recorder(Arc<Mutex<Vec<u32>>>);

impl Backend for Recorder {
    fn compile(&mut self, _: &Sb) -> Option<u32> {
        None
    }
    fn release(&mut self, id: u32) {
        self.0.lock().unwrap().push(id);
    }
    fn name(&self) -> &'static str {
        "recorder"
    }
    fn run(&mut self, _: u32, _: &mut Cpu, _: &mut SysBus, _: &mut u64) -> RunOut {
        RunOut {
            exit: EXIT_SLOW,
            target: 0,
        }
    }
}

fn with_recorder(w: &mut World) -> Arc<Mutex<Vec<u32>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    w.st.backend = Some(Box::new(Recorder(log.clone())));
    log
}

#[test]
fn a_dropped_trace_releases_its_backend_code_once() {
    let mut w = World::new();
    let log = with_recorder(&mut w);
    w.mem.write32(0x1000, 1);
    w.mem.write32(0x1800, 2);
    let (a, b) = (w.trace(0x1000, &[0x1000]), w.trace(0x1800, &[0x1800]));
    w.st.cache.get_mut(a).native = 7;
    w.st.cache.get_mut(b).native = 8;
    w.mem.write32(0x1000, 11);
    w.drain();
    assert_eq!(*log.lock().unwrap(), vec![7]);
    assert_eq!(
        (w.st.cache.get(a).native, w.st.cache.get(b).native),
        (NO_SB, 8)
    );
    w.st.drop_trace(a);
    assert_eq!(log.lock().unwrap().len(), 1, "not released twice");
}

#[test]
fn the_cache_is_flushed_when_it_goes_over_its_uop_cap_and_not_before() {
    let mut w = World::new();
    w.st.cfg.uop_cap = 3;
    let log = with_recorder(&mut w);
    w.st.hot.bump(0x1000);
    w.st.hot.mark_failed(0x1004);
    let epoch = w.st.cache.epoch();
    for i in 0..3u32 {
        let id =
            w.st.cache
                .insert(tiny_trace(0x1000 + 4 * i, &[(0x1000 + 4 * i, i)]));
        w.st.cache.get_mut(id).native = 10 + i;
    }
    assert!(!w.st.flush_if_over_cap(), "3 uops against a cap of 3");
    let id = w.st.cache.insert(tiny_trace(0x2000, &[(0x2000, 0)]));
    w.st.cache.get_mut(id).native = 13;
    assert!(w.st.flush_if_over_cap(), "4 uops");
    assert_eq!(
        (
            w.st.cache.live_count(),
            w.st.cache.uops_total(),
            w.st.stats.flushes
        ),
        (0, 0, 1)
    );
    assert_eq!(*log.lock().unwrap(), vec![10, 11, 12, 13]);
    assert_ne!(w.st.cache.epoch(), epoch);
    assert_eq!(
        (w.st.hot.bump(0x1000), w.st.hot.failed(0x1004)),
        (2, true),
        "hotness is kept"
    );
    assert!(!w.st.flush_if_over_cap());
}

#[test]
fn cloning_the_state_gives_a_cold_cache_with_the_same_configuration() {
    let mut w = World::new();
    w.mem.write32(0x1000, 1);
    w.trace(0x1000, &[0x1000]);
    w.st.cache.mark_volatile(4);
    w.st.hot.bump(0x1000);
    w.st.hot.mark_failed(0x1008);
    w.st.stats.built = 2;
    w.st.cfg.hot = 9;
    let c = w.st.clone();
    assert_eq!((c.cfg.hot, c.cfg.enabled, c.placeholder), (9, true, false));
    assert_eq!(
        (c.cache.live_count(), c.cache.slots(), c.stats.built),
        (0, 0, 0)
    );
    assert!(c.cache.lookup(0x1000).is_none() && c.cache.covering(1).is_empty());
    assert!(!c.cache.volatile(4) && !c.hot.failed(0x1008) && c.pending.is_empty());
    let mut c = c;
    assert_eq!(c.hot.bump(0x1000), 1, "its counts start from zero");
    assert_eq!(w.st.cache.live_count(), 1, "and the original is untouched");
}
