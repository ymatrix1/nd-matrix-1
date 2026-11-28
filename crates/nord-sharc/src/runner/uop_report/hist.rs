//! The block-length histogram of design 11.3 ("520 blocks per pass at 12 steps each is ASSUMED"):
//! how many steps each block entry ran, bucketed. The transition cost of the model
//! (`model::TRANSITION`) scales with the number of entries, so the 9.1 total is recomputed from
//! this table at M3.
//!
//! OWNER: W1-RUN. `UopStats` has no per-length counters (it is frozen), and the translated blocks
//! are private to `core::uop` with no read-only accessor, so the table is filled from what the
//! driver counted around each block entry (`uop_pass::lean_pass_uop_entries`, untimed): the length
//! is the steps the entry ran, which is the block length when it ran whole and less when it
//! exited early (`Slow`, `Smc`, `Jump` in the middle). A static table of the translated blocks
//! fills the same type through [`BlockHistogram::from_lengths`] once the core offers an accessor.
use crate::runner::uop_pass::BlockEntry;

/// Upper bounds (inclusive, in steps) of the buckets; one more bucket takes everything longer.
/// ASSUMED: fine at the short end, where the design expects blocks cut at every flow, `Sys` and
/// loop end; the last edge is the default `UopConfig::max_block_steps` (48, itself ASSUMED), so
/// the open bucket holds only entries that ran past a block (a chain). Confirm with the table of
/// a real core: if most entries land in one bucket the edges can be coarsened.
pub const EDGES: [u32; 11] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48];
/// Number of buckets: one per edge and the open one.
pub const BUCKETS: usize = EDGES.len() + 1;

/// Steps per block entry, bucketed by [`EDGES`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockHistogram {
    counts: [u64; BUCKETS],
    entries: u64,
    steps: u64,
    chained: u64,
    empty: u64,
}

impl BlockHistogram {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bucket a length of `steps` (at least 1) falls in.
    pub fn bucket_of(steps: u64) -> usize {
        EDGES
            .iter()
            .position(|&e| steps <= u64::from(e))
            .unwrap_or(EDGES.len())
    }

    /// Records one block that ran `steps` steps (a zero length is an [`BlockHistogram::empty`]
    /// entry, not a block).
    pub fn add(&mut self, steps: u64) {
        if steps == 0 {
            self.empty += 1;
            return;
        }
        self.counts[Self::bucket_of(steps)] += 1;
        self.entries += 1;
        self.steps += steps;
    }

    /// Records one block entry of the driver; an "entry" that found no block (`blocks == 0`) is
    /// not one and is ignored. An entry that ran more than one block (a chain of linked blocks)
    /// cannot be split into lengths: it is counted apart, in [`Self::chained`].
    pub fn add_entry(&mut self, e: &BlockEntry) {
        match e.blocks {
            0 => {}
            1 => self.add(e.steps),
            _ => self.chained += 1,
        }
    }

    /// A table of known block lengths (the translated blocks, when a caller can list them).
    pub fn from_lengths(lengths: impl IntoIterator<Item = u32>) -> Self {
        let mut h = Self::new();
        for l in lengths {
            h.add(u64::from(l));
        }
        h
    }

    /// Entries per bucket, shortest first ([`BUCKETS`] of them).
    pub fn counts(&self) -> &[u64; BUCKETS] {
        &self.counts
    }

    /// Entries bucketed (each ran one block and at least one step).
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Steps of the bucketed entries.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Entries that ran more than one linked block and so are in no bucket.
    pub fn chained(&self) -> u64 {
        self.chained
    }

    /// Block entries that ran no step (the block bailed on its first uop: a `Slow` or `Guard`
    /// exit before any work) and so are in no bucket. `entries + empty` plus the blocks of the
    /// chained entries (at least 2 each) add up to the engine's `blocks_run`, so
    /// `entries + empty + 2 * chained <= blocks_run`, with equality when nothing chained
    /// (`tools/uop_gates.py` checks the inequality on a report).
    pub fn empty(&self) -> u64 {
        self.empty
    }

    /// Mean steps per bucketed entry, 0 for an empty table.
    pub fn mean(&self) -> f64 {
        if self.entries == 0 {
            0.0
        } else {
            self.steps as f64 / self.entries as f64
        }
    }

    /// The range of bucket `i` as text: `1`, `5-6`, `49+`.
    pub fn label(i: usize) -> String {
        let hi = EDGES.get(i).copied();
        let lo = if i == 0 { 1 } else { EDGES[i - 1] + 1 };
        match hi {
            None => format!("{lo}+"),
            Some(hi) if hi == lo => format!("{lo}"),
            Some(hi) => format!("{lo}-{hi}"),
        }
    }

    /// One line for a person: the non-empty buckets, then the totals.
    pub fn text(&self) -> String {
        let buckets: Vec<String> = self
            .counts
            .iter()
            .enumerate()
            .filter(|(_, &c)| c > 0)
            .map(|(i, c)| format!("{}:{c}", Self::label(i)))
            .collect();
        format!(
            "steps per block entry [{}] ({} entries, {} steps, mean {:.1}, {} chained, {} empty)",
            if buckets.is_empty() {
                "none".to_string()
            } else {
                buckets.join(" ")
            },
            self.entries,
            self.steps,
            self.mean(),
            self.chained,
            self.empty,
        )
    }

    /// The table as a JSON object (field names stable with the report schema).
    pub fn to_json(&self) -> String {
        let list = |v: &[u64]| v.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let edges: Vec<u64> = EDGES.iter().map(|&e| u64::from(e)).collect();
        format!(
            "{{\"edges\":[{}],\"counts\":[{}],\"entries\":{},\"steps\":{},\"chained\":{},\"empty\":{}}}",
            list(&edges),
            list(&self.counts),
            self.entries,
            self.steps,
            self.chained,
            self.empty
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_fall_in_the_bucket_whose_range_holds_them() {
        for (steps, bucket) in [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 3),
            (5, 4),
            (6, 4),
            (7, 5),
            (8, 5),
            (9, 6),
            (12, 6),
            (13, 7),
            (48, 10),
            (49, 11),
            (10_000, 11),
        ] {
            assert_eq!(BlockHistogram::bucket_of(steps), bucket, "{steps} steps");
        }
        assert_eq!(BUCKETS, 12);
    }

    #[test]
    fn labels_name_the_range_of_each_bucket_and_the_edges_tile_without_gaps() {
        let labels: Vec<String> = (0..BUCKETS).map(BlockHistogram::label).collect();
        assert_eq!(
            labels,
            ["1", "2", "3", "4", "5-6", "7-8", "9-12", "13-16", "17-24", "25-32", "33-48", "49+"]
        );
        // every length is in exactly the bucket whose label range holds it
        for steps in 1..=100u64 {
            let l = BlockHistogram::label(BlockHistogram::bucket_of(steps));
            let (lo, hi) = match l.split_once('-') {
                Some((a, b)) => (a.parse::<u64>().unwrap(), b.parse::<u64>().unwrap()),
                None if l.ends_with('+') => (l.trim_end_matches('+').parse().unwrap(), u64::MAX),
                None => (l.parse().unwrap(), l.parse().unwrap()),
            };
            assert!((lo..=hi).contains(&steps), "{steps} in {l}");
        }
    }

    #[test]
    fn a_table_counts_entries_steps_and_the_mean() {
        let mut h = BlockHistogram::new();
        assert_eq!((h.entries(), h.mean()), (0, 0.0));
        for s in [1, 1, 12, 12, 12, 100] {
            h.add(s);
        }
        assert_eq!((h.entries(), h.steps()), (6, 138));
        assert_eq!(h.mean(), 23.0);
        let want = {
            let mut c = [0u64; BUCKETS];
            c[0] = 2;
            c[6] = 3;
            c[11] = 1;
            c
        };
        assert_eq!(h.counts(), &want);
        assert_eq!(h.counts().iter().sum::<u64>(), h.entries());
    }

    #[test]
    fn a_driver_entry_of_one_block_is_bucketed_and_the_others_are_counted_apart() {
        let entry = |blocks, steps| BlockEntry {
            blocks,
            uops: steps,
            steps,
        };
        let mut h = BlockHistogram::new();
        h.add_entry(&entry(1, 5));
        h.add_entry(&entry(1, 0));
        h.add_entry(&entry(0, 0)); // no block was found: not an entry
        h.add_entry(&entry(3, 30));
        assert_eq!(
            (h.entries(), h.steps(), h.chained(), h.empty()),
            (1, 5, 1, 1)
        );
        assert_eq!(h.counts()[BlockHistogram::bucket_of(5)], 1);
    }

    #[test]
    fn known_block_lengths_fill_the_same_table() {
        let h = BlockHistogram::from_lengths([1, 12, 12, 48, 60]);
        assert_eq!((h.entries(), h.steps()), (5, 133));
        let mut by_hand = BlockHistogram::new();
        for s in [1, 12, 12, 48, 60] {
            by_hand.add(s);
        }
        assert_eq!(h, by_hand);
    }

    #[test]
    fn the_text_lists_the_non_empty_buckets_and_the_totals() {
        let h = BlockHistogram::from_lengths([1, 1, 12, 49]);
        assert_eq!(
            h.text(),
            "steps per block entry [1:2 9-12:1 49+:1] (4 entries, 63 steps, mean 15.8, 0 chained, 0 empty)"
        );
        assert!(BlockHistogram::new().text().contains("[none] (0 entries"));
    }

    #[test]
    fn the_json_is_one_object_with_the_edges_next_to_the_counts() {
        let j = BlockHistogram::from_lengths([1, 12]).to_json();
        assert_eq!(
            j,
            "{\"edges\":[1,2,3,4,6,8,12,16,24,32,48],\"counts\":[1,0,0,0,0,0,1,0,0,0,0,0],\
\"entries\":2,\"steps\":13,\"chained\":0,\"empty\":0}"
        );
        assert!(!j.contains('\n'));
    }
}
