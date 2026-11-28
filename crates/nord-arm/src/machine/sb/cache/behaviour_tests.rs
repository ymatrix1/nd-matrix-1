//! OWNER: W1-CACHE. Behaviour tests of [`SbCache`]: replacement, covering lists, flush, caps and
//! the reset of links to dead traces.

use super::*;
use crate::machine::sb::uop::{Exit, ExitKind};

#[test]
fn the_last_word_of_a_page_and_the_first_of_the_next_are_distinct_entries() {
    let mut c = SbCache::default();
    let a = c.insert(tiny_trace(0x1FFC, &[(0x1FFC, 1)]));
    let b = c.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
    assert_eq!((c.lookup(0x1FFC), c.lookup(0x2000)), (Some(a), Some(b)));
    assert_eq!(c.lookup(0x1000), None);
}

#[test]
fn a_second_trace_at_the_same_entry_replaces_the_first() {
    let mut c = SbCache::default();
    let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
    let b = c.insert(tiny_trace(0x1000, &[(0x1000, 2)]));
    assert_ne!(a, b);
    assert_eq!(
        (c.lookup(0x1000), c.is_live(a), c.live_count()),
        (Some(b), false, 1)
    );
    assert_eq!(c.covering(1), &[b]);
}

#[test]
fn covering_lists_the_live_traces_that_read_a_page_in_insertion_order() {
    let mut c = SbCache::default();
    let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1), (0x5000, 2)]));
    let b = c.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
    let d = c.insert(tiny_trace(0x1800, &[(0x1800, 1), (0x2004, 3)]));
    assert_eq!(c.covering(1), &[a, d]);
    assert_eq!(c.covering(5), &[a]);
    assert_eq!(c.covering(2), &[b, d]);
    assert!(c.covering(9).is_empty() && c.covering(0xF_FFFF).is_empty());
    c.kill(a);
    assert!(c.covering(5).is_empty());
    assert_eq!(c.covering(1), &[d]);
}

#[test]
fn a_misaligned_entry_is_tracked_but_never_found() {
    let mut c = SbCache::default();
    let id = c.insert(tiny_trace(0x1002, &[(0x1000, 1)]));
    assert_eq!((c.lookup(0x1002), c.lookup(0x1000)), (None, None));
    assert_eq!(c.covering(1), &[id]);
    c.kill(id);
    assert!(c.covering(1).is_empty());
}

#[test]
fn flush_empties_the_cache_and_volatile_pages_are_remembered() {
    let mut c = SbCache::default();
    c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
    c.mark_volatile(3);
    assert_eq!(c.note_change(1), 1);
    let epoch = c.epoch();
    c.flush();
    assert_eq!(
        (c.live_count(), c.uops_total(), c.lookup(0x1000), c.slots()),
        (0, 0, None, 0)
    );
    assert!(c.volatile(3) && !c.volatile(4) && c.covering(1).is_empty());
    assert_eq!(c.epoch(), epoch.wrapping_add(1));
    assert_eq!(c.note_change(1), 2, "the counter survives");
    assert_eq!(
        c.insert(tiny_trace(0x1000, &[(0x1000, 1)])),
        0,
        "ids restart"
    );
}

#[test]
fn the_change_counter_saturates() {
    let mut c = SbCache::default();
    let last = (0..300).map(|_| c.note_change(7)).last();
    assert_eq!(last, Some(255));
}

#[test]
fn the_caps_ask_for_a_flush_on_uops_and_on_slots() {
    let mut c = SbCache::default();
    c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
    c.insert(tiny_trace(0x1004, &[(0x1004, 1)]));
    assert_eq!((c.over_cap(2), c.over_cap(1)), (false, true));
    assert!(
        std::mem::size_of::<Sb>() * SLOT_CAP <= 12 << 20,
        "the SLOT_CAP provenance note says 12 MB at the cap"
    );
    for i in 0..SLOT_CAP as u32 {
        let id = c.insert(tiny_trace(0x2000, &[(0x2000, i)]));
        c.kill(id);
    }
    assert!(c.over_cap(usize::MAX), "dead slots alone trigger it");
}

#[test]
fn links_to_dead_traces_are_reset_and_live_ones_kept() {
    let mut c = SbCache::default();
    let a = c.insert(tiny_trace(0x1000, &[(0x1000, 1)]));
    let b = c.insert(tiny_trace(0x2000, &[(0x2000, 1)]));
    let d = c.insert(tiny_trace(0x3000, &[(0x3000, 1)]));
    let mut x = Exit::new(ExitKind::Indirect, 1, 0x1000, 0);
    x.link = b;
    x.ic_id = [b, d];
    x.ic_pc = [0x2000, 0x3000];
    let mut sb = tiny_trace(0x4000, &[(0x4000, 1)]);
    sb.exits = vec![Exit::new(ExitKind::Direct, 1, 0x4000, 0x1000), x].into_boxed_slice();
    let e = c.insert(sb);
    c.get_mut(e).exits[0].link = a;
    assert_eq!(c.reset_dead_links(), 0, "nothing is dead yet");
    c.kill(b);
    assert_eq!(
        c.reset_dead_links(),
        2,
        "the link and the first cache entry"
    );
    let exits = &c.get(e).exits;
    assert_eq!((exits[0].link, exits[1].link), (a, NO_SB));
    assert_eq!((exits[1].ic_id, exits[1].ic_pc), ([NO_SB, d], [0, 0x3000]));
    c.kill(e);
    assert_eq!(c.reset_dead_links(), 0, "dead traces are not walked");
}
