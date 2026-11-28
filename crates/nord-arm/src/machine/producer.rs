//! ARM0 on a decoupled producer thread (`ns4-lle2` tail follow-up, D251).
//!
//! The ARM core runs ahead of virtual time on its own thread and hands each tick's messages to the
//! consumer (the DSP side) through a bounded queue of `queue_ticks` ticks. A tick that costs more
//! than 1 ms of host time does not stall the consumer as long as the producer is at least that far
//! ahead, so the real-time criterion becomes "the backlog never exceeds the queue", not "every
//! tick under 1 ms" ([`queue_ticks_for`] sizes the queue from a measured per-tick cost series).
//!
//! * **Deterministic mode** ([`collect_blocking`]): the consumer pulls with a blocking `recv`; no
//!   wall clock is read anywhere. The message stream is a function of the machine, the budget, the
//!   tick count and the input script only: two runs are identical whatever the thread scheduling
//!   or the queue size (test below).
//! * **Real-time mode** ([`consume_realtime`]): the consumer takes ready ticks once per ms of wall
//!   clock and counts the ticks that were not ready at their deadline (underruns). The producer is
//!   the same code: it simply runs until the queue is full (back-pressure: `send` blocks, counted
//!   in [`Report::full_events`] and [`Report::blocked`]).
//! * **Input**: a script of `(tick, bytes)` entries is applied at the start of that tick
//!   (deterministic). Live input through [`Producer::input`] is applied at the start of the
//!   producer's next tick and recorded in [`Report::applied`] with the tick it landed on, so a
//!   live session can be replayed as a script.
//!
//! The producer allocates per tick (the message `Vec`); that is the ARM thread, not the audio
//! thread. The consumer returns drained buffers through [`Producer::recycle`], so the audio side
//! neither allocates nor frees.

use super::grouper::Message;
use super::idle::IdleStats;
use super::metrics::tail;
use super::{Machine, Stop};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, TryRecvError, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Messages one tick produced.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickMsgs {
    pub tick: u64,
    pub msgs: Vec<Message>,
}

/// What a producer run needs.
#[derive(Debug, Clone)]
pub struct Config {
    /// Instructions of virtual time per tick (1 ms).
    pub budget: u64,
    /// Ticks the queue holds (at least 1).
    pub queue_ticks: usize,
    /// Ticks to produce.
    pub ticks: u64,
    /// `(tick, MIDI bytes)` applied at the start of `tick`; need not be sorted.
    pub script: Vec<(u64, Vec<u8>)>,
}

/// What the producer thread returns when it ends.
pub struct Report {
    pub machine: Machine,
    pub stop: Stop,
    pub produced: u64,
    /// Ticks that found the queue full (the producer had to wait for the consumer).
    pub full_events: u64,
    /// Wall clock spent blocked on a full queue.
    pub blocked: Duration,
    /// Every input applied, with the tick it was applied at (script entries and live input).
    pub applied: Vec<(u64, Vec<u8>)>,
    pub idle: IdleStats,
}

/// Handle of a running producer.
pub struct Producer {
    pub rx: Receiver<TickMsgs>,
    input: Sender<Vec<u8>>,
    recycle: Sender<Vec<Message>>,
    join: JoinHandle<Report>,
}

impl Producer {
    /// Start `machine` on its own thread.
    pub fn spawn(mut machine: Machine, mut cfg: Config) -> Producer {
        let (tx, rx) = sync_channel::<TickMsgs>(cfg.queue_ticks.max(1));
        let (input, input_rx) = channel::<Vec<u8>>();
        let (recycle, recycle_rx) = channel::<Vec<Message>>();
        cfg.script.sort_by_key(|s| s.0);
        let join = std::thread::spawn(move || {
            let mut idle = IdleStats::default();
            let (mut applied, mut script, mut produced) = (Vec::new(), cfg.script.iter(), 0u64);
            let (mut full_events, mut blocked, mut stop) = (0u64, Duration::ZERO, Stop::Budget);
            let mut next = script.next();
            'ticks: for tick in 0..cfg.ticks {
                while let Ok(bytes) = input_rx.try_recv() {
                    machine.send_midi(&bytes);
                    applied.push((tick, bytes));
                }
                while let Some(s) = next.filter(|s| s.0 <= tick) {
                    machine.send_midi(&s.1);
                    applied.push((tick, s.1.clone()));
                    next = script.next();
                }
                let s = machine.run_tick_idle(cfg.budget, &mut idle);
                if !matches!(s, Stop::Budget) {
                    stop = s;
                    break;
                }
                let mut msgs = recycle_rx.try_recv().unwrap_or_default();
                msgs.clear();
                machine.drain_messages_into(&mut msgs);
                let mut out = TickMsgs { tick, msgs };
                match tx.try_send(out) {
                    Ok(()) => {}
                    Err(TrySendError::Full(t)) => {
                        full_events += 1;
                        out = t;
                        let t0 = Instant::now();
                        let sent = tx.send(out);
                        blocked += t0.elapsed();
                        if sent.is_err() {
                            break 'ticks;
                        }
                    }
                    Err(TrySendError::Disconnected(_)) => break 'ticks,
                }
                produced += 1;
            }
            Report {
                machine,
                stop,
                produced,
                full_events,
                blocked,
                applied,
                idle,
            }
        });
        Producer {
            rx,
            input,
            recycle,
            join,
        }
    }

    /// Live MIDI input: applied at the start of the producer's next tick.
    pub fn input(&self, bytes: Vec<u8>) {
        let _ = self.input.send(bytes);
    }

    /// Give a drained buffer back to the producer for reuse.
    pub fn recycle(&self, buf: Vec<Message>) {
        let _ = self.recycle.send(buf);
    }

    /// Wait for the producer thread (drop `rx` first if the consumer stops early).
    pub fn finish(self) -> Report {
        drop(self.rx);
        self.join.join().expect("producer thread panicked")
    }
}

/// Deterministic consumer: blocking `recv` until the producer ends, no clock. Returns the ticks in
/// arrival order (always `0, 1, 2, ...`).
pub fn collect_blocking(p: &Producer) -> Vec<TickMsgs> {
    let mut out = Vec::new();
    while let Ok(t) = p.rx.recv() {
        out.push(t);
    }
    out
}

/// What the real-time consumer saw.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RealtimeReport {
    /// Ticks taken, in order.
    pub taken: u64,
    /// Deadlines (tick `k` is due `k + 1` ms after `t0`) at which the tick was not ready yet.
    pub underruns: u64,
    /// Largest lateness of a tick behind its deadline, ms (0 when never late).
    pub max_late_ms: f64,
    /// Most ticks that were ready (buffered) at one deadline.
    pub max_ready: usize,
}

/// Real-time consumer: at each deadline `t0 + (k + 1) ms` take every tick that is ready; a tick
/// that is not ready counts as an underrun and is taken as soon as it arrives. `t0` in the past
/// makes every deadline already due (tests).
pub fn consume_realtime(p: &Producer, t0: Instant, ticks: u64) -> RealtimeReport {
    let mut rep = RealtimeReport::default();
    let mut ready = Vec::new();
    while rep.taken < ticks {
        let due = t0 + Duration::from_micros(1000 * (rep.taken + 1));
        while Instant::now() < due {
            std::hint::spin_loop();
        }
        let before = rep.taken;
        loop {
            match p.rx.try_recv() {
                Ok(t) => {
                    debug_assert_eq!(t.tick, rep.taken);
                    rep.taken += 1;
                    ready.push(t.msgs);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return rep,
            }
        }
        rep.max_ready = rep.max_ready.max((rep.taken - before) as usize);
        if rep.taken == before {
            rep.underruns += 1;
            // wait for the tick (it is late); lateness is measured against its deadline
            match p.rx.recv() {
                Ok(t) => {
                    rep.taken += 1;
                    ready.push(t.msgs);
                    let late = due.elapsed().as_secs_f64() * 1e3;
                    rep.max_late_ms = rep.max_late_ms.max(late);
                }
                Err(_) => return rep,
            }
        }
        for b in ready.drain(..) {
            p.recycle(b);
        }
    }
    rep
}

/// Queue size in ticks that absorbs the measured backlog of a series of per-tick costs in ms
/// (consumed one per ms, [`tail`]): `ceil(1.5 * worst backlog) + 2`.
///
/// ASSUMED: the 1.5 margin and the 2 ticks of slack (the tick the producer is blocked sending and
/// one of jitter). Evidence: a queue of `ceil(backlog) + 2` taken from ONE run underran in 4 of 5
/// real-time runs at 150 k (host jitter), none at 200 k and above; feed it the worst series of
/// several runs.
pub fn queue_ticks_for(cost_ms: &[f64]) -> usize {
    let (_, backlog) = tail(cost_ms);
    (backlog * 1.5).ceil() as usize + 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::sysbus::PORT1_BASE;

    /// Synthetic ARM0: counts in r2 and stores the count as a DSP message every iteration.
    fn machine() -> Machine {
        let code = [
            0xE282_2001, // add r2,r2,#1
            0xE588_2030, // str r2,[r8,#0x30]  (first word of comm 3, path 1)
            0xE5C8_2031, // strb r2,[r8,#0x31] (second word completes the message)
            0xEAFF_FFFB, // b .-12 (back to the add)
        ];
        let mut m = Machine::new();
        for (i, w) in code.iter().enumerate() {
            m.bus.mem.write32(0xC000_0000 + 4 * i as u32, *w);
        }
        m.cpu.r[8] = PORT1_BASE;
        m.cpu.r[15] = 0xC000_0000;
        m
    }

    fn cfg(queue: usize, ticks: u64) -> Config {
        Config {
            budget: 400,
            queue_ticks: queue,
            ticks,
            script: vec![(5, vec![0x90, 60, 100]), (3, vec![0x80, 60, 0])],
        }
    }

    fn run(queue: usize, ticks: u64, slow_consumer: bool) -> (Vec<TickMsgs>, Report) {
        let p = Producer::spawn(machine(), cfg(queue, ticks));
        let mut got = Vec::new();
        while let Ok(t) = p.rx.recv() {
            if slow_consumer {
                std::thread::sleep(Duration::from_micros(300));
            }
            got.push(t);
        }
        (got, p.finish())
    }

    #[test]
    fn two_runs_produce_identical_message_streams_whatever_the_queue_or_the_consumer() {
        let (a, ra) = run(1, 40, false);
        let (b, rb) = run(64, 40, false);
        let (c, rc) = run(2, 40, true);
        assert_eq!(a.len(), 40);
        assert!(a.iter().map(|t| t.msgs.len()).sum::<usize>() > 100);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(
            (ra.machine.state_hash(), ra.produced),
            (rb.machine.state_hash(), 40)
        );
        assert_eq!(ra.machine.state_hash(), rc.machine.state_hash());
        assert!(a.iter().enumerate().all(|(i, t)| t.tick == i as u64));
    }

    #[test]
    fn script_input_lands_on_its_tick_in_order() {
        let (_, r) = run(4, 10, false);
        assert_eq!(
            r.applied,
            vec![(3, vec![0x80, 60, 0]), (5, vec![0x90, 60, 100])]
        );
    }

    #[test]
    fn a_full_queue_blocks_the_producer_and_is_counted() {
        let (got, r) = run(1, 30, true);
        assert_eq!(got.len(), 30);
        assert!(
            r.full_events > 0,
            "a 1-tick queue behind a slow consumer fills"
        );
        assert!(r.blocked > Duration::ZERO);
        let (_, big) = run(64, 30, false);
        assert_eq!(big.full_events, 0, "a queue as long as the run never fills");
    }

    #[test]
    fn live_input_is_applied_at_a_tick_start_and_recorded_for_replay() {
        let p = Producer::spawn(machine(), cfg(2, 20));
        p.input(vec![0x90, 64, 90]);
        let got = collect_blocking(&p);
        let r = p.finish();
        assert_eq!(got.len(), 20);
        let live: Vec<_> = r.applied.iter().filter(|a| a.1 == [0x90, 64, 90]).collect();
        assert_eq!(live.len(), 1, "{:?}", r.applied);
        assert!(live[0].0 < 20);
    }

    #[test]
    fn realtime_consumer_counts_underruns_and_takes_every_tick_in_order() {
        // start time far in the past: every deadline is already due, so the run is not paced
        let t0 = Instant::now().checked_sub(Duration::from_secs(5)).unwrap();
        let p = Producer::spawn(machine(), cfg(8, 50));
        let rep = consume_realtime(&p, t0, 50);
        let r = p.finish();
        assert_eq!(rep.taken, 50);
        assert_eq!(r.produced, 50);
        assert!(rep.max_ready <= 8 + 2, "{rep:?}");
        assert!(rep.max_late_ms.is_finite());
    }

    #[test]
    fn queue_size_follows_the_backlog() {
        // a 5 ms tick behind 1 ms ticks: 4 ms of backlog, times 1.5, plus the slack
        let mut ms = vec![0.2; 10];
        assert_eq!(queue_ticks_for(&ms), 2);
        ms[3] = 5.0;
        assert_eq!(queue_ticks_for(&ms), 8);
    }
}
