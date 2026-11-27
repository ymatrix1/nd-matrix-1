//! Real-time pacing of the ARM0 tick loop (`ns4-lle2` follow-up 2): a producer runs ticks at one
//! per millisecond of wall clock on its own thread and hands each tick's messages to a consumer
//! through a bounded queue; the consumer drains the queue once per millisecond like an audio
//! callback would. Measures message latency, queue depth and producer stalls with real threads
//! (both sides spin on the clock: Windows sleeps are too coarse for 1 ms).

use super::grouper::Message;
use super::idle::IdleStats;
use super::Machine;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

/// One finished tick on its way to the consumer.
pub struct Stamp {
    pub tick: usize,
    /// Wall clock at completion, in ms after the run start.
    pub done_ms: f64,
    pub msgs: Vec<Message>,
}

/// Ticks in flight between the producer and the consumer, counted on both sides.
///
/// The producer calls [`Flight::enter`] just before `send` (so a tick blocked in `send` counts) and
/// the consumer calls [`Flight::leave`] just after `recv`. The peak is therefore at most the
/// channel capacity (buffered) + 1 (the producer's tick blocked in `send`) + 1 (received, not yet
/// counted out): a bound that does not depend on thread scheduling, unlike the number of ticks one
/// consumer drain happens to see (a late consumer lets the producer catch up with overdue ticks
/// while the drain loop is still running).
#[derive(Debug, Default)]
pub struct Flight {
    cur: AtomicUsize,
    max: AtomicUsize,
}

impl Flight {
    pub fn enter(&self) {
        let now = self.cur.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
    }

    pub fn leave(&self) {
        self.cur.fetch_sub(1, Ordering::SeqCst);
    }

    /// Peak number of ticks in flight so far.
    pub fn max(&self) -> usize {
        self.max.load(Ordering::SeqCst)
    }
}

/// What the consumer saw.
#[derive(Debug, Default, Clone)]
pub struct PacedReport {
    /// Per tick (index = tick number): `(lateness in ms, message count)`. Lateness is the drain
    /// time minus the end of the tick's virtual millisecond, never negative.
    pub per_tick: Vec<(f64, usize)>,
    /// Most messages / ticks handed over in one drain loop. Not bounded by the channel capacity:
    /// ticks the producer sends while the loop runs count too (see [`Flight`] for the bound).
    pub max_queue_msgs: usize,
    pub max_queue_ticks: usize,
    /// Peak ticks in flight (buffered + blocked in `send`), from [`Flight::max`].
    pub max_inflight: usize,
    /// Ticks that arrived in an order other than 0, 1, 2, ... (0 when the queue is FIFO).
    pub out_of_order: usize,
    /// Wall clock the producer spent blocked in `send` on a full queue, ms.
    pub blocked_ms: f64,
}

fn wait_until(t0: Instant, ms: f64) {
    let at = t0 + Duration::from_secs_f64(ms / 1e3);
    while Instant::now() < at {
        std::hint::spin_loop();
    }
}

/// Run `ticks` ticks of `tick_insns` instructions, tick `first + i` released `first + i` ms after
/// `t0`. `action` runs inside the first tick (MIDI input). Returns the ms spent blocked in `send`.
/// `flight` counts the ticks handed to the queue (shared with [`consume`]).
#[allow(clippy::too_many_arguments)]
pub fn produce(
    m: &mut Machine,
    stats: &mut IdleStats,
    t0: Instant,
    first: usize,
    ticks: usize,
    tick_insns: u64,
    action: fn(&mut Machine),
    tx: &SyncSender<Stamp>,
    flight: &Flight,
) -> f64 {
    let mut blocked = Duration::ZERO;
    for i in 0..ticks {
        let tick = first + i;
        wait_until(t0, tick as f64);
        if i == 0 {
            action(m);
        }
        m.run_tick_idle(tick_insns, stats);
        let mut msgs = Vec::new();
        m.drain_messages_into(&mut msgs);
        let done_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        flight.enter();
        if tx
            .send(Stamp {
                tick,
                done_ms,
                msgs,
            })
            .is_err()
        {
            flight.leave();
            break;
        }
        blocked += t.elapsed();
    }
    blocked.as_secs_f64() * 1e3
}

/// Drain `rx` once per millisecond until `total` ticks arrived.
pub fn consume(rx: &Receiver<Stamp>, t0: Instant, total: usize, flight: &Flight) -> PacedReport {
    let mut rep = PacedReport {
        per_tick: vec![(0.0, 0); total],
        ..Default::default()
    };
    let mut got = 0;
    let mut block = 1.0f64;
    while got < total {
        wait_until(t0, block);
        let now_ms = t0.elapsed().as_secs_f64() * 1e3;
        let (mut msgs, mut ticks) = (0, 0);
        while let Ok(s) = rx.try_recv() {
            flight.leave();
            if s.tick != got {
                rep.out_of_order += 1;
            }
            rep.per_tick[s.tick] = ((now_ms - (s.tick + 1) as f64).max(0.0), s.msgs.len());
            msgs += s.msgs.len();
            ticks += 1;
            got += 1;
        }
        rep.max_queue_msgs = rep.max_queue_msgs.max(msgs);
        rep.max_queue_ticks = rep.max_queue_ticks.max(ticks);
        block = (now_ms.floor() + 1.0).max(block + 1.0);
    }
    rep.max_inflight = flight.max();
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;
    use std::sync::Arc;

    const TICKS: usize = 20;
    const CAP: usize = 4;
    const INSNS: u64 = 1000;
    /// Buffered ticks plus the one the producer is blocked sending.
    const FULL: usize = CAP + 1;

    /// An idle machine: `b .` at the load base, no firmware.
    fn idle_machine() -> Machine {
        let mut m = Machine::new();
        m.bus.mem.write32(0xC000_0000, 0xEAFF_FFFE);
        m.cpu.r[15] = 0xC000_0000;
        m
    }

    /// Per-tick message counts of the same machine run on one thread, no queue, no clock.
    fn reference_counts() -> Vec<usize> {
        let (mut m, mut stats) = (idle_machine(), IdleStats::default());
        (0..TICKS)
            .map(|_| {
                m.run_tick_idle(INSNS, &mut stats);
                let mut msgs = Vec::new();
                m.drain_messages_into(&mut msgs);
                msgs.len()
            })
            .collect()
    }

    /// A start time in the past: every tick is already due, so no thread waits on the clock.
    fn long_ago() -> Instant {
        let now = Instant::now();
        now.checked_sub(Duration::from_secs(1)).unwrap_or(now)
    }

    fn spawn_producer(
        t0: Instant,
        tx: SyncSender<Stamp>,
        flight: Arc<Flight>,
    ) -> std::thread::JoinHandle<f64> {
        let mut m = idle_machine();
        std::thread::spawn(move || {
            let mut stats = IdleStats::default();
            produce(
                &mut m,
                &mut stats,
                t0,
                0,
                TICKS,
                INSNS,
                |_| {},
                &tx,
                &flight,
            )
        })
    }

    #[test]
    fn paced_ticks_arrive_in_order_with_their_message_counts_and_a_bounded_queue() {
        // Real pacing (both threads wait on the clock). Everything asserted here holds under any
        // thread scheduling: lateness and the ticks one drain sees are host-dependent (a late
        // wake lets the producer catch up while the drain loop runs) and are not asserted.
        let (tx, rx) = sync_channel(CAP);
        let flight = Arc::new(Flight::default());
        let t0 = Instant::now() + Duration::from_millis(2);
        let handle = spawn_producer(t0, tx, flight.clone());
        let rep = consume(&rx, t0, TICKS, &flight);
        let blocked = handle.join().unwrap();
        assert_eq!(rep.per_tick.len(), TICKS);
        assert_eq!(rep.out_of_order, 0, "{rep:?}");
        let counts: Vec<usize> = rep.per_tick.iter().map(|p| p.1).collect();
        assert_eq!(counts, reference_counts());
        assert!(rep.max_inflight <= CAP + 2, "{rep:?}");
        assert!(blocked >= 0.0);
        assert!(
            rep.per_tick.iter().all(|(l, _)| l.is_finite() && *l >= 0.0),
            "{rep:?}"
        );
    }

    #[test]
    fn a_consumer_that_does_not_drain_holds_the_producer_at_the_queue_capacity() {
        // Deterministic: the start time is in the past, so the producer never waits on the clock
        // and runs until the queue stops it. It can only get CAP ticks buffered plus the one it is
        // blocked sending; that holds until this thread drains, whatever the scheduling.
        let (tx, rx) = sync_channel(CAP);
        let flight = Arc::new(Flight::default());
        let t0 = long_ago();
        let handle = spawn_producer(t0, tx, flight.clone());
        let deadline = Instant::now() + Duration::from_secs(60);
        while flight.max() < FULL {
            assert!(
                Instant::now() < deadline,
                "the producer never filled the queue"
            );
            std::thread::yield_now();
        }
        // nothing was received yet, so the count cannot pass CAP + 1; give a faulty unbounded
        // queue time to show it (this wait can only make the assertion stricter, never flaky)
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            flight.max(),
            CAP + 1,
            "the queue did not hold the producer back"
        );
        let rep = consume(&rx, t0, TICKS, &flight);
        let blocked = handle.join().unwrap();
        assert_eq!(rep.per_tick.len(), TICKS);
        assert_eq!(rep.out_of_order, 0, "{rep:?}");
        assert!((FULL..=FULL + 1).contains(&rep.max_inflight), "{rep:?}");
        assert!(
            blocked > 0.0,
            "the producer was blocked in send for the 20 ms wait"
        );
    }
}
