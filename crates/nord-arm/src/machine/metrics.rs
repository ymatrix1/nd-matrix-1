//! Wall-clock metrics of a tick run (`ns4-lle2` follow-ups): percentiles, the over-budget tail,
//! and a model of a decoupled ARM0 thread that hands its messages to an audio consumer through a
//! queue. Pure functions on per-tick timings, so the benchmark binary stays small and the maths
//! has unit tests.

use super::grouper::Message;

/// Value at rank `floor((n - 1) * p)` of the sorted samples (`p` in `0..=1`).
pub fn percentile(v: &[f64], p: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f64 * p) as usize]
}

/// Longest run of consecutive ticks over 1 ms, and the largest backlog (ms of work behind real
/// time) if ticks were consumed at 1 per ms with leftover work carried over.
pub fn tail(ms: &[f64]) -> (usize, f64) {
    let (mut run, mut longest, mut backlog, mut worst) = (0usize, 0usize, 0.0f64, 0.0f64);
    for t in ms {
        if *t > 1.0 {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
        backlog = (backlog + t - 1.0).max(0.0);
        worst = worst.max(backlog);
    }
    (longest, worst)
}

/// FNV-1a style digest of a message list (path, comm, words, pc of every message in order), to
/// compare two runs without printing the lists.
pub fn digest_messages(msgs: &[Message]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for m in msgs {
        for w in [m.path as u32, m.comm as u32, m.word1, m.word2, m.pc] {
            h = (h ^ w as u64).wrapping_mul(0x100_0000_01B3);
        }
    }
    h
}

/// Message-weighted latency percentiles `[p50, p99, p99.9, max]` of `(latency, messages)` pairs
/// (all zero when there are no messages).
pub fn weighted_percentiles(lat: &[(f64, usize)]) -> [f64; 4] {
    let mut sorted = lat.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let total: usize = sorted.iter().map(|x| x.1).sum();
    if total == 0 {
        return [0.0; 4];
    }
    [0.5, 0.99, 0.999, 1.0].map(|p| {
        let rank = ((total - 1) as f64 * p) as usize;
        let mut seen = 0usize;
        for (l, n) in &sorted {
            seen += n;
            if seen > rank {
                return *l;
            }
        }
        sorted.last().map_or(0.0, |x| x.0)
    })
}

/// Result of [`simulate_queue`].
#[derive(Debug, Clone, PartialEq)]
pub struct QueueReport {
    /// Message latency percentiles in ms (message-weighted): p50, p99, p99.9, max.
    pub latency: [f64; 4],
    /// Ticks whose messages reached the consumer later than their ideal time.
    pub late_ticks: usize,
    /// Most messages waiting in the queue at any consumer drain.
    pub max_queue: usize,
    /// Most ticks completing between two consumer drains.
    pub max_ticks_per_block: usize,
}

/// Model of a decoupled ARM0 thread feeding an audio consumer.
///
/// Tick `i` is released at `i` ms and its virtual time ends at `i + 1` ms. The ARM thread runs
/// ticks in order, each starting at `max(previous completion, release)` and taking `ms[i]`, so a
/// slow tick delays the next ones until they catch up. The consumer drains the queue once per
/// block of `block_ms` (all messages that completed before the drain). A message of tick `i`
/// has the ideal delivery time `i + 1` ms; its latency is the drain time minus that, never
/// negative. `msgs[i]` is the number of messages tick `i` produced.
pub fn simulate_queue(ms: &[f64], msgs: &[usize], block_ms: f64) -> QueueReport {
    assert_eq!(ms.len(), msgs.len());
    let mut done = Vec::with_capacity(ms.len());
    let mut t = 0.0f64;
    for (i, d) in ms.iter().enumerate() {
        t = t.max(i as f64) + d;
        done.push(t);
    }
    // (latency, message count) per tick, and the drain time of each tick's messages
    let mut lat: Vec<(f64, usize)> = Vec::with_capacity(ms.len());
    let mut drains: Vec<f64> = Vec::with_capacity(ms.len());
    for (i, c) in done.iter().enumerate() {
        let drain = (c / block_ms).ceil() * block_ms;
        let ideal = (i + 1) as f64;
        lat.push(((drain - ideal).max(0.0), msgs[i]));
        drains.push(drain);
    }
    let late_ticks = lat.iter().filter(|(l, _)| *l > 1e-9).count();
    let latency = weighted_percentiles(&lat);
    // queue depth at each drain: messages and ticks sharing a drain time
    let (mut max_queue, mut max_ticks) = (0usize, 0usize);
    let (mut i, n) = (0usize, drains.len());
    while i < n {
        let (mut j, mut q) = (i, 0usize);
        while j < n && (drains[j] - drains[i]).abs() < 1e-9 {
            q += msgs[j];
            j += 1;
        }
        max_queue = max_queue.max(q);
        max_ticks = max_ticks.max(j - i);
        i = j;
    }
    QueueReport {
        latency,
        late_ticks,
        max_queue,
        max_ticks_per_block: max_ticks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_picks_sorted_rank() {
        let v = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 0.5), 3.0);
        assert_eq!(percentile(&v, 1.0), 5.0);
    }

    #[test]
    fn tail_finds_the_longest_run_and_the_backlog() {
        // 3.0, 3.0 over by 2 each: backlog 4, then two 0.2 ms ticks repay 0.8 each
        let (run, backlog) = tail(&[0.2, 3.0, 3.0, 0.2, 0.2, 0.2]);
        assert_eq!(run, 2);
        assert!((backlog - 4.0).abs() < 1e-9, "{backlog}");
        assert_eq!(tail(&[0.5, 0.9]), (0, 0.0));
    }

    #[test]
    fn weighted_percentiles_count_messages_not_ticks() {
        // 9 messages at 0 ms, 1 at 5 ms: only the maximum sees the slow one
        let p = weighted_percentiles(&[(5.0, 1), (0.0, 9)]);
        assert_eq!(p, [0.0, 0.0, 0.0, 5.0]);
        assert_eq!(weighted_percentiles(&[(3.0, 0)]), [0.0; 4]);
    }

    #[test]
    fn digest_depends_on_every_field_and_the_order() {
        let a = Message {
            path: 1,
            comm: 2,
            word1: 3,
            word2: 4,
            pc: 5,
        };
        let b = Message { pc: 6, ..a };
        assert_ne!(digest_messages(&[a]), digest_messages(&[b]));
        assert_ne!(digest_messages(&[a, b]), digest_messages(&[b, a]));
        assert_eq!(digest_messages(&[a, b]), digest_messages(&[a, b]));
    }

    #[test]
    fn queue_with_fast_ticks_has_no_latency() {
        let r = simulate_queue(&[0.3; 10], &[2; 10], 1.0);
        assert_eq!(r.latency, [0.0; 4]);
        assert_eq!(
            (r.late_ticks, r.max_queue, r.max_ticks_per_block),
            (0, 2, 1)
        );
    }

    #[test]
    fn a_slow_tick_delays_the_next_ones_and_bursts_the_queue() {
        // tick 0 takes 3.5 ms (done 3.5), tick 1 starts at 3.5 and takes 0.2 (3.7), tick 2 (3.9)
        let r = simulate_queue(&[3.5, 0.2, 0.2, 0.2], &[10, 1, 1, 1], 1.0);
        // drains at 4: ticks 0..3 all complete by 4.1? tick 3 done 4.1 -> drain 5
        assert_eq!(r.max_ticks_per_block, 3);
        assert_eq!(r.max_queue, 12);
        // tick 0 ideal 1 ms, drained at 4 ms: 3 ms late, the 10 messages dominate every percentile
        assert!((r.latency[3] - 3.0).abs() < 1e-9, "{:?}", r.latency);
        assert_eq!(r.late_ticks, 4);
    }
}
