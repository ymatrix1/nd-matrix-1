//! Firmware chorus (Mod 2 `CHOR`, `kFx2Type` 4 and 5): memory map and driver of the
//! instruction-level ports [`super::fw_chorus4`] (routine `0x122E24`) and
//! [`super::fw_chorus5`] (`0x122F77`).
//!
//! The routines keep a phase word, an odd-polynomial LFO, smoothed delay times and a circular
//! stereo delay line (`L = 0x200` words with a mirrored guard copy) in the data memory of the
//! FX DSP; the ARM allocates the ring and writes its base and the running pointer into the
//! context. Here the context words, the ring and one small frame block are three arrays at the
//! addresses of the model scenarios (`tools/fx_model_chorus.py`), so a context captured from the
//! DSP loads unchanged and the written-back words compare word for word with the golden vectors.
//!
//! Provenance (D133): the direct-call scenarios of `docs/research/fx-mod2-types.md` missed the
//! left wet tap (a wrapped pre-modify store wrote no guard copy), so that old claim is withdrawn.
//! EMPIRICAL: the live chain of the oracle (`sharc-fxlive`, real dispatcher) against this port,
//! `python tools/fx_oracle_live.py chor4|chor5 --image <core1_sram.bin> --signal noise`: null depth
//! -119 to -156 dB for CHOR soft over rates 0..127 and amounts 0..127, and -89.5 dB for the
//! variation at rate 64 / amount 64 only (the oracle context of `chor5` is fixed; the residual
//! cause is not measured). The check needs the local firmware image, so no test in the repo
//! compares against the oracle; `tests_chorus_golden.rs` pins the port to the Python twin only.

use super::fw_pe::{Mem, Pe};
use super::{fw_chorus4, fw_chorus5};

/// `I1` of the context (any base works: the routine only uses offsets from it).
pub(super) const CTX_I1: u32 = 0xB2040;
/// First context word (`I1 - 0x20`) and number of words read or written (`I1 - 0x20 ..= I1 + 0x25`).
pub(super) const CTX_LO: u32 = CTX_I1 - 0x20;
pub(super) const CTX_N: usize = 0x46;
/// Base of the delay ring and its length in words (`0x200` ring plus the mirrored guard words).
pub(super) const RING_BASE: u32 = 0xB4000;
pub(super) const RING_N: usize = 0x410;
/// Frame block (`I0`): up to [`MAX_BLOCKS`] blocks of two stereo frames.
const IO_BASE: u32 = 0xB1000;
pub(super) const MAX_BLOCKS: usize = 4;
/// Scratch pairs the routines park the dry frames in (absolute data addresses).
const SCRATCH: u32 = 0xC285D;

/// Which routine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChorusKind {
    /// `kFx2Type` 4, routine `0x122E24`.
    Soft,
    /// `kFx2Type` 5, routine `0x122F77`.
    Variation,
}

impl ChorusKind {
    /// Context offset of the block count word (`lcntr`).
    pub(super) const fn blocks_offset(self) -> i32 {
        match self {
            Self::Soft => 9,
            Self::Variation => 2,
        }
    }
}

/// The three memory regions of one call.
pub(super) struct ChorusMem<'a> {
    ctx: &'a mut [u32; CTX_N],
    ring: &'a mut [u32; RING_N],
    io: [u32; 4 * MAX_BLOCKS],
    scratch: [u32; 4],
}

fn slot(addr: u32, lo: u32, n: usize) -> Option<usize> {
    let k = addr.wrapping_sub(lo) as usize;
    (k < n).then_some(k)
}

impl Mem for ChorusMem<'_> {
    fn rd(&self, a: u32) -> u32 {
        if let Some(k) = slot(a, CTX_LO, CTX_N) {
            self.ctx[k]
        } else if let Some(k) = slot(a, RING_BASE, RING_N) {
            self.ring[k]
        } else if let Some(k) = slot(a, IO_BASE, self.io.len()) {
            self.io[k]
        } else if let Some(k) = slot(a, SCRATCH, 4) {
            self.scratch[k]
        } else {
            0
        }
    }

    fn wr(&mut self, a: u32, w: u32) {
        if let Some(k) = slot(a, CTX_LO, CTX_N) {
            self.ctx[k] = w;
        } else if let Some(k) = slot(a, RING_BASE, RING_N) {
            self.ring[k] = w;
        } else if let Some(k) = slot(a, IO_BASE, self.io.len()) {
            self.io[k] = w;
        } else if let Some(k) = slot(a, SCRATCH, 4) {
            self.scratch[k] = w;
        }
    }
}

/// Firmware chorus: context words and delay ring. Allocation free (the arrays are inline).
#[derive(Debug, Clone)]
pub(super) struct FwChorus {
    kind: ChorusKind,
    ctx: [u32; CTX_N],
    ring: [u32; RING_N],
}

impl FwChorus {
    /// Builds the chorus from the context words (`I1 - 0x20 ..= I1 + 0x25`) and the ring.
    pub(super) fn new(kind: ChorusKind, ctx: [u32; CTX_N], ring: [u32; RING_N]) -> Self {
        Self { kind, ctx, ring }
    }

    /// The routine this instance runs.
    pub(super) fn kind(&self) -> ChorusKind {
        self.kind
    }

    /// Context word at `offset` from `I1`.
    pub(super) fn word(&self, offset: i32) -> u32 {
        self.ctx[(offset + 0x20) as usize]
    }

    /// Sets the context word at `offset` from `I1`.
    pub(super) fn set_word(&mut self, offset: i32, word: u32) {
        self.ctx[(offset + 0x20) as usize] = word;
    }

    /// Sets a float context word.
    pub(super) fn set_value(&mut self, offset: i32, v: f32) {
        self.set_word(offset, v.to_bits());
    }

    /// The context words including the written-back state.
    #[cfg(test)]
    pub(super) fn words(&self) -> &[u32; CTX_N] {
        &self.ctx
    }

    /// The delay ring words.
    #[cfg(test)]
    pub(super) fn ring(&self) -> &[u32; RING_N] {
        &self.ring
    }

    /// One call on `frames = [l0, r0, l1, r1, ...]` in place: whole blocks of two stereo frames
    /// (a trailing partial block is left untouched). The firmware's block count word is set to
    /// the number of blocks.
    pub(super) fn process(&mut self, frames: &mut [f32]) {
        let n = (frames.len() / 4).min(MAX_BLOCKS);
        self.set_word(self.kind.blocks_offset(), n as u32);
        let mut io = [0u32; 4 * MAX_BLOCKS];
        for (w, f) in io.iter_mut().zip(frames.iter()).take(4 * n) {
            *w = f.to_bits();
        }
        let mem = ChorusMem {
            ctx: &mut self.ctx,
            ring: &mut self.ring,
            io,
            scratch: [0; 4],
        };
        let mut pe = Pe::new(mem, IO_BASE, CTX_I1);
        match self.kind {
            ChorusKind::Soft => fw_chorus4::run(&mut pe),
            ChorusKind::Variation => fw_chorus5::run(&mut pe),
        }
        for (f, w) in frames.iter_mut().zip(pe.mem.io).take(4 * n) {
            *f = f32::from_bits(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fw_words::assert_close;
    use super::super::tests_chorus_golden as g;
    use super::*;

    /// The deterministic ring of `tools/fx_golden_mod2.py::ring_word`.
    pub(super) fn golden_ring() -> [u32; RING_N] {
        std::array::from_fn(|k| {
            let h = (k as u32).wrapping_mul(2_654_435_761).wrapping_add(12345);
            ((h >> 9) as f32 / 4_194_304.0 - 1.0).to_bits()
        })
    }

    /// Runs a golden: three calls of two blocks, then the final context and the changed ring words.
    fn check(
        kind: ChorusKind,
        ctx_in: &[u32],
        fin: &[u32],
        fout: &[u32],
        ctx_out: &[u32],
        diff: &[(u32, u32)],
    ) {
        let mut c = FwChorus::new(kind, ctx_in.try_into().unwrap(), golden_ring());
        for (n, (i, o)) in fin.chunks(8).zip(fout.chunks(8)).enumerate() {
            let mut frames: Vec<f32> = i.iter().map(|w| f32::from_bits(*w)).collect();
            c.process(&mut frames);
            let got: Vec<u32> = frames.iter().map(|f| f.to_bits()).collect();
            assert_close(&got, o, 5.0e-6, &format!("{kind:?} call {n}"));
        }
        // The context words carry the f32 rounding of three calls on the live ring (the model keeps
        // extended precision): 2e-5 relative, the frames stay within 5e-6.
        assert_close(c.words(), ctx_out, 2.0e-5, "final context");
        let want = golden_ring();
        let changed: Vec<usize> = (0..RING_N).filter(|&k| c.ring()[k] != want[k]).collect();
        assert_eq!(
            changed,
            diff.iter().map(|d| d.0 as usize).collect::<Vec<_>>(),
            "the same ring words are written"
        );
        let got: Vec<u32> = diff.iter().map(|d| c.ring()[d.0 as usize]).collect();
        let exp: Vec<u32> = diff.iter().map(|d| d.1).collect();
        assert_close(&got, &exp, 5.0e-6, "ring words");
    }

    #[test]
    fn the_soft_chorus_matches_the_firmware_model() {
        check(
            ChorusKind::Soft,
            &g::CH4_CTX_IN,
            &g::CH4_FRAMES_IN,
            &g::CH4_FRAMES_OUT,
            &g::CH4_CTX_OUT,
            &g::CH4_RING_DIFF,
        );
    }

    #[test]
    fn the_variation_chorus_matches_the_firmware_model() {
        check(
            ChorusKind::Variation,
            &g::CH5_CTX_IN,
            &g::CH5_FRAMES_IN,
            &g::CH5_FRAMES_OUT,
            &g::CH5_CTX_OUT,
            &g::CH5_RING_DIFF,
        );
    }

    #[test]
    fn the_golden_vectors_are_not_silent() {
        assert!(g::CH4_FRAMES_IN != g::CH4_FRAMES_OUT && g::CH5_FRAMES_IN != g::CH5_FRAMES_OUT);
        assert!(g::CH4_CTX_IN != g::CH4_CTX_OUT && g::CH5_RING_DIFF.len() > 1);
    }

    #[test]
    fn memory_outside_the_regions_reads_zero_and_ignores_writes() {
        let (mut ctx, mut ring) = ([0u32; CTX_N], [0u32; RING_N]);
        let mut m = ChorusMem {
            ctx: &mut ctx,
            ring: &mut ring,
            io: [0; 16],
            scratch: [0; 4],
        };
        m.wr(0x1234, 7);
        assert_eq!(m.rd(0x1234), 0);
        m.wr(SCRATCH + 3, 5);
        assert_eq!(m.rd(SCRATCH + 3), 5);
    }

    /// Regression for the pre-modify store `DM(M6, I6)` (`I6 + 0x202`, no wrap, D114): the ports
    /// wrapped it, so the guard copy of the ring was never written and the left wet tap of an
    /// impulse stayed silent while the oracle (live FX chain, `docs/research/fx-reverb0-chor-oracle-diff.md`)
    /// plays it (port against oracle after the fix: 80 calls, max difference 9.2e-7).
    fn impulse_through(variation: bool) -> (Vec<[f32; 2]>, [u32; RING_N]) {
        let mut st = super::super::fw_mod2::FwMod2Stage::new(variation, 64, 64, 43_402.344);
        let out: Vec<[f32; 2]> = (0..400)
            .map(|i| {
                let x = if i == 0 { 0.5 } else { 0.0 };
                st.process_frame([x, 0.8 * x])
            })
            .collect();
        (out, *st.ring_for_test())
    }

    #[test]
    fn an_impulse_has_a_left_and_a_right_wet_tap_and_the_guard_copy() {
        for variation in [false, true] {
            let (out, ring) = impulse_through(variation);
            let wet = |ch: usize| out[40..].iter().map(|f| f[ch].abs()).fold(0.0f32, f32::max);
            assert!(
                wet(0) > 1.0e-4,
                "variation {variation}: left wet {}",
                wet(0)
            );
            assert!(
                wet(1) > 1.0e-4,
                "variation {variation}: right wet {}",
                wet(1)
            );
            let mirrored = (0..0x200)
                .filter(|&k| ring[k] != 0 && ring[k + 0x200] == ring[k])
                .count();
            assert!(
                mirrored >= 1,
                "variation {variation}: guard copy written ({mirrored} words)"
            );
        }
    }
}
