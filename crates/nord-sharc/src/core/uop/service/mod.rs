//! Translation service of the uop engine: the lock-free rings and the worker (design 6.3, 7.4).
//!
//! OWNER: W1-SVC. This file only declares the tree and re-exports the two ring types the native
//! tier names.
pub mod ring;
pub mod worker;

pub use ring::Ring;
pub use worker::{Service, TransDone, TransReq};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reexports_name_the_types_of_the_submodules() {
        let mut r: Ring<TransReq, 2> = Ring::new();
        let (mut p, mut c) = r.split();
        p.push(TransReq {
            lo: 1,
            hi: 2,
            words: Box::new([0x0001]),
        })
        .unwrap();
        assert_eq!(c.pop().map(|q| (q.lo, q.hi)), Some((1, 2)));
        assert!(Service::new().poll().is_none());
    }
}
