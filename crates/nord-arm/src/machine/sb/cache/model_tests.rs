//! OWNER: W1-CACHE. Tests of [`SbCache`] against a plain reference model, and of its
//! allocation behaviour. Deterministic: a fixed xorshift stream drives every case.

use super::{tiny_trace, SbCache};
use crate::machine::sb::uop::SbId;

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// What the model remembers of one trace.
struct Slot {
    entry: u32,
    pages: Vec<u32>,
    live: bool,
}

/// Entries and read words drawn from 6 pages and 8 word positions, so replacements, shared pages
/// and traces that read several pages all happen often.
fn random_trace(r: &mut Xorshift) -> (u32, Vec<(u32, u32)>) {
    let word = |r: &mut Xorshift| ((r.below(6) as u32) << 12) | ((r.below(8) as u32) << 2);
    let entry = word(r);
    let mut words = vec![(entry, 0)];
    for _ in 0..r.below(3) {
        words.push((word(r), 1));
    }
    (entry, words)
}

fn check(c: &SbCache, model: &[Slot]) {
    let live: Vec<SbId> = (0..model.len() as SbId)
        .filter(|id| model[*id as usize].live)
        .collect();
    assert_eq!(c.live_count(), live.len());
    assert_eq!(c.uops_total(), live.len(), "one uop per tiny trace");
    assert_eq!(c.iter_live().map(|(id, _)| id).collect::<Vec<_>>(), live);
    for id in 0..model.len() as SbId {
        assert_eq!(c.is_live(id), model[id as usize].live, "id {id}");
    }
    for page in 0..7u32 {
        let want: Vec<SbId> = live
            .iter()
            .copied()
            .filter(|id| model[*id as usize].pages.contains(&page))
            .collect();
        assert_eq!(c.covering(page), want, "page {page}");
    }
    for page in 0..6u32 {
        for w in 0..8u32 {
            let pc = (page << 12) | (w << 2);
            let want = live
                .iter()
                .copied()
                .find(|id| model[*id as usize].entry == pc);
            assert_eq!(c.lookup(pc), want, "pc {pc:#x}");
        }
    }
}

#[test]
fn a_random_sequence_of_inserts_kills_and_flushes_agrees_with_the_model() {
    let mut r = Xorshift(0x2545_F491_4F6C_DD1D);
    let mut c = SbCache::default();
    let mut model: Vec<Slot> = Vec::new();
    let (mut inserts, mut replaced, mut kills, mut flushes, mut multi_page) = (0, 0, 0, 0, 0);
    for _ in 0..4000 {
        match r.below(20) {
            0..=11 => {
                let (entry, words) = random_trace(&mut r);
                if model.iter().any(|s| s.live && s.entry == entry) {
                    replaced += 1;
                    for s in model.iter_mut().filter(|s| s.live && s.entry == entry) {
                        s.live = false;
                    }
                }
                let mut pages: Vec<u32> = words.iter().map(|(a, _)| a >> 12).collect();
                pages.sort_unstable();
                pages.dedup();
                multi_page += usize::from(pages.len() > 1);
                let id = c.insert(tiny_trace(entry, &words));
                assert_eq!(id as usize, model.len(), "ids are the arena order");
                model.push(Slot {
                    entry,
                    pages,
                    live: true,
                });
                inserts += 1;
            }
            12..=18 => {
                let id = r.below(model.len() as u64 + 1) as SbId;
                kills += usize::from(model.get(id as usize).is_some_and(|s| s.live));
                c.kill(id);
                if let Some(s) = model.get_mut(id as usize) {
                    s.live = false;
                }
            }
            _ => {
                c.flush();
                model.clear();
                flushes += 1;
            }
        }
        check(&c, &model);
    }
    assert!(
        inserts > 2000 && replaced > 100 && kills > 300 && flushes > 100 && multi_page > 500,
        "the run must exercise every operation: {inserts} {replaced} {kills} {flushes} {multi_page}"
    );
}

#[test]
fn lookup_covering_and_kill_never_allocate_once_a_page_is_known() {
    let mut c = SbCache::default();
    // Fill every list to its working size first: the inserts may grow the page lists.
    let ids: Vec<SbId> = (0..64u32)
        .map(|i| {
            c.insert(tiny_trace(
                0x1000 + 4 * i,
                &[(0x1000 + 4 * i, 0), (0x2000, 1)],
            ))
        })
        .collect();
    let probe = |c: &SbCache| (c.pages.buffers(), c.arena.as_ptr(), c.arena.capacity());
    let lists = |c: &SbCache| (c.covering(1).as_ptr(), c.covering(2).as_ptr());
    let (before, lists_before) = (probe(&c), lists(&c));
    for _ in 0..50 {
        for i in 0..64u32 {
            assert!(c.lookup(0x1000 + 4 * i).is_some());
        }
        assert_eq!((c.covering(1).len(), c.covering(2).len()), (64, 64));
    }
    assert_eq!(probe(&c), before, "reads leave every buffer where it was");
    for id in ids.iter().rev().step_by(2) {
        c.kill(*id);
    }
    assert_eq!(
        probe(&c),
        before,
        "kills free bodies but never reallocate the tables"
    );
    assert_eq!(lists(&c), lists_before, "the page lists shrink in place");
    assert_eq!(c.covering(2).len(), 32);
}
