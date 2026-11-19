//! Address hasher and sparse word store of the peripheral and external-bus window of [`Core`].
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Hasher for the device store: addresses are `u32` keys that are not attacker controlled, so a
/// multiplicative mix is enough and much cheaper than the default SipHash on the poll loops.
#[derive(Default)]
pub(super) struct AddrHasher(u64);

impl Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, x: u32) {
        self.0 = u64::from(x).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// Sparse word store keyed by address.
pub(super) type DeviceMap = HashMap<u32, u32, BuildHasherDefault<AddrHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_hasher_spreads_neighbouring_addresses() {
        use std::hash::BuildHasher;
        let build = BuildHasherDefault::<AddrHasher>::default();
        let hashes: std::collections::HashSet<u64> = (0..1024u32)
            .map(|k| build.hash_one(0x3108_C000 + 4 * k))
            .collect();
        assert_eq!(hashes.len(), 1024);
        let mut h = AddrHasher::default();
        h.write(&[1, 2, 3]);
        assert_ne!(h.finish(), 0);
    }
}
