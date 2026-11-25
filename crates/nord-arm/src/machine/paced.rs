//! Real-time pacing of the ARM0 tick loop (`ns4-lle2` follow-up 2): a producer runs ticks at one
//! per millisecond of wall clock on its own thread and hands each tick's messages to a consumer
//! through a bounded queue; the consumer drains the queue once per millisecond like an audio
//! callback would. Measures message latency, queue depth and producer stalls with real threads
//! (both sides spin on the clock: Windows sleeps are too coarse for 1 ms).

use super::grouper::Message;
use super::idle::IdleStats;
use super::Machine;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

/// One finished tick on its way to the consumer.
pub struct Stamp {
    pub tick: usize,
    /// Wall clock at completion, in ms after the run start.
    pub done_ms: f64,
    pub msgs: Vec<Message>,
}

/// What the consumer saw.
#[derive(Debug, Default, Clone)]
pub struct PacedReport {
    /// Per tick (index = tick number): `(lateness in ms, message count)`. Lateness is the drain
    /// time minus the end of the tick's virtual millisecond, never negative.
    pub per_tick: Vec<(f64, usize)>,
    /// Most messages / ticks handed over in one drain (the queue depth just before it).
    pub max_queue_msgs: usize,
    pub max_queue_ticks: usize,
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
        if tx
            .send(Stamp {
                tick,
                done_ms,
                msgs,
            })
            .is_err()
        {
            break;
        }
        blocked += t.elapsed();
    }
    blocked.as_secs_f64() * 1e3
}

/// Drain `rx` once per millisecond until `total` ticks arrived.
pub fn consume(rx: &Receiver<Stamp>, t0: Instant, total: usize) -> PacedReport {
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
            rep.per_tick[s.tick] = ((now_ms - (s.tick + 1) as f64).max(0.0), s.msgs.len());
            msgs += s.msgs.len();
            ticks += 1;
            got += 1;
        }
        rep.max_queue_msgs = rep.max_queue_msgs.max(msgs);
        rep.max_queue_ticks = rep.max_queue_ticks.max(ticks);
        block = (now_ms.floor() + 1.0).max(block + 1.0);
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    #[test]
    fn paced_ticks_arrive_in_order_with_small_lateness_and_the_queue_is_bounded() {
        // an idle machine: `b .` at the load base, no firmware
        let mut m = Machine::new();
        m.bus.mem.write32(0xC000_0000, 0xEAFF_FFFE);
        m.cpu.r[15] = 0xC000_0000;
        let (tx, rx) = sync_channel(4);
        let t0 = Instant::now() + Duration::from_millis(2);
        let handle = std::thread::spawn(move || {
            let mut stats = IdleStats::default();
            produce(&mut m, &mut stats, t0, 0, 20, 1000, |_| {}, &tx)
        });
        let rep = consume(&rx, t0, 20);
        let blocked = handle.join().unwrap();
        assert_eq!(rep.per_tick.len(), 20);
        assert!(rep.max_queue_ticks <= 5, "{rep:?}");
        assert!(blocked >= 0.0);
        // a loop of 1000 instructions finishes in microseconds: nothing is a whole ms late
        // (generous bound: a loaded CI host can stall a thread for a few ms)
        assert!(rep.per_tick.iter().all(|(l, _)| *l < 50.0), "{rep:?}");
    }
}
