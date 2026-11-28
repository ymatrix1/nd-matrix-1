//! The translation service: requests in, translated blocks out, through two rings.
//!
//! OWNER: W1-SVC (thread, request ring); the native validation of what comes back is W4-NSVC.
//! A request carries the code words of a range so the worker never touches a `Core`; the answer
//! carries blocks whose `first` indexes are relative to its own `uops` (the contract of
//! `lower::translate_range`), installed by `UopHot::install` at a pass boundary.
//!
//! Skeleton: the service is synchronous. `pump` plays the worker on the calling thread with a
//! caller-supplied translator, which is what the tests and `UopConfig::async_translate == false`
//! use; the worker thread replaces `pump` and keeps the same two rings.
use super::ring::Ring;
use crate::core::uop::types::{Block, Uop};

/// Slots of each ring. ASSUMED: a handful of ranges are in flight at once (program changes).
pub const CAP: usize = 16;

/// A request to translate the short words `[lo, hi)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransReq {
    pub lo: u32,
    pub hi: u32,
    /// The code words `lo..hi` plus the tail of the last instruction, copied at request time.
    pub words: Box<[u16]>,
}

/// The answer to one request.
#[derive(Clone, Debug)]
pub struct TransDone {
    pub blocks: Vec<Block>,
    pub uops: Vec<Uop>,
}

/// The request and answer rings of one engine.
#[derive(Default)]
pub struct Service {
    req: Ring<TransReq, CAP>,
    done: Ring<TransDone, CAP>,
}

impl Service {
    /// An idle service.
    pub fn new() -> Self {
        Service::default()
    }

    /// Queues a request, or gives it back when [`CAP`] are already waiting.
    pub fn submit(&mut self, req: TransReq) -> Result<(), TransReq> {
        self.req.split().0.push(req)
    }

    /// Takes the oldest finished translation.
    pub fn poll(&mut self) -> Option<TransDone> {
        self.done.split().1.pop()
    }

    /// Translates every queued request with `translate` on the calling thread and queues the
    /// answers (an answer that does not fit is dropped and not counted). Returns the number of
    /// requests served.
    pub fn pump(&mut self, mut translate: impl FnMut(TransReq) -> TransDone) -> usize {
        let mut served = 0;
        while let Some(req) = self.req.split().1.pop() {
            if self.done.split().0.push(translate(req)).is_ok() {
                served += 1;
            }
        }
        served
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(lo: u32) -> TransReq {
        TransReq {
            lo,
            hi: lo + 4,
            words: vec![0x0001; 4].into_boxed_slice(),
        }
    }

    #[test]
    fn requests_are_served_in_order_and_answers_come_back_in_order() {
        let mut s = Service::new();
        assert!(s.poll().is_none());
        for lo in [0x12_0000, 0x12_0100, 0x12_0200] {
            s.submit(req(lo)).unwrap();
        }
        let served = s.pump(|r| TransDone {
            blocks: vec![Block::empty(r.lo)],
            uops: Vec::new(),
        });
        assert_eq!(served, 3);
        let pcs: Vec<u32> = std::iter::from_fn(|| s.poll())
            .map(|d| d.blocks[0].pc)
            .collect();
        assert_eq!(pcs, [0x12_0000, 0x12_0100, 0x12_0200]);
    }

    #[test]
    fn a_full_request_ring_gives_the_request_back() {
        let mut s = Service::new();
        for k in 0..CAP as u32 {
            s.submit(req(k)).unwrap();
        }
        assert_eq!(s.submit(req(99)).unwrap_err().lo, 99);
        assert_eq!(
            s.pump(|_| TransDone {
                blocks: Vec::new(),
                uops: Vec::new()
            }),
            CAP
        );
    }
}
