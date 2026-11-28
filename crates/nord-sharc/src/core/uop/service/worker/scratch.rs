//! The scratch core of the translation worker: a private `Core` that holds the words of one
//! request and nothing else, so the worker never touches the engine's own core.
//!
//! OWNER: W1-SVC. A translation reads the code through the decode path of `Core::step`
//! (`lower::decode::decode_at`), so it needs a `Core`; the request carries the parcels instead
//! (`TransReq::words`). [`Scratch::translate`] puts them at the short-word address they came
//! from, in the same place `Core::parcel_at` looks, translates with `lower::translate_range` and
//! hands the answer back, which is exactly what `translate_range` returns on the live core for
//! the same words. The scratch core has no peripheral, no ISA image, no open interrupt window and
//! PEy off, so `translate_range` must derive everything from the words, the config and
//! `stop_pc` (it translates both PEy modes itself, design P4). A request is self-contained: the
//! previous request's words are cleared and its decode-cache entries are invalidated first.
//!
//! The words are a copy made at request time: a store into the code after that makes the answer
//! stale, which the installer detects by comparing `Block::code_hash` with the live core
//! (`lower::decode::code_hash`, design P6).
use super::{TransDone, TransReq};
use crate::core::uop::config::{UopConfig, UopError};
use crate::core::uop::lower::translate_range;
use crate::core::uop::types::{Block, Uop};
use crate::core::{Core, SRAM_BYTES};

/// The signature of `lower::translate_range`. The native service and the tests pass their own.
pub(crate) type Translate =
    fn(&mut Core, &UopConfig, u32, u32) -> Result<(Vec<Block>, Vec<Uop>), UopError>;

/// A private core and the words it currently holds.
pub struct Scratch {
    core: Core,
    /// Short-word address and parcel count of the words mounted by the last request.
    mounted: (u32, usize),
}

/// Writes `parcels` at the short-word address `at`, through the byte mapping `Core::parcel_at`
/// reads (little-endian parcels, the SRAM index wraps at [`SRAM_BYTES`]).
fn put_parcels(core: &mut Core, at: u32, parcels: impl Iterator<Item = u16>) {
    for (k, parcel) in parcels.enumerate() {
        let o = (at.wrapping_add(k as u32) as usize) * 2;
        let [lo, hi] = parcel.to_le_bytes();
        core.sram[o & (SRAM_BYTES - 1)] = lo;
        core.sram[(o + 1) & (SRAM_BYTES - 1)] = hi;
    }
}

impl Scratch {
    /// A scratch core with an empty code window. `stop_pc` is the pass idle pc the translator
    /// makes a block start (design P7), or `None`. Allocates the core's memory (4 MiB).
    pub fn new(stop_pc: Option<u32>) -> Self {
        let mut core = Core::new(&[]);
        core.uop_set_stop_pc(stop_pc);
        Scratch {
            core,
            mounted: (0, 0),
        }
    }

    /// Changes the stop pc of the following requests.
    pub fn set_stop_pc(&mut self, stop_pc: Option<u32>) {
        self.core.uop_set_stop_pc(stop_pc);
    }

    /// What `lower::translate_range` returns for the words of `req`, translated on the scratch
    /// core. `OutOfRange` when the range is empty or `req.words` does not cover it.
    pub fn translate(&mut self, cfg: &UopConfig, req: &TransReq) -> Result<TransDone, UopError> {
        self.translate_with(translate_range, cfg, req)
    }

    /// [`Scratch::translate`] with another translator over the same mounted words.
    pub(crate) fn translate_with(
        &mut self,
        translator: Translate,
        cfg: &UopConfig,
        req: &TransReq,
    ) -> Result<TransDone, UopError> {
        let (lo, hi) = (req.lo, req.hi);
        let span = hi.checked_sub(lo).filter(|&n| n > 0);
        if span.is_none_or(|n| req.words.len() < n as usize) {
            return Err(UopError::OutOfRange { lo, hi });
        }
        // clear the last request, then mount this one; the generation bump makes every decoded
        // entry of the last request a miss
        let (old_at, old_len) = self.mounted;
        put_parcels(&mut self.core, old_at, std::iter::repeat_n(0, old_len));
        put_parcels(&mut self.core, lo, req.words.iter().copied());
        self.mounted = (lo, req.words.len());
        self.core.invalidate_code_cache();
        let (blocks, uops) = translator(&mut self.core, cfg, lo, hi)?;
        Ok(TransDone { blocks, uops })
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{probe, program, render};
    use super::*;
    use crate::core::support::{core_with, CODE};
    use crate::core::uop::types::NONE;

    fn req(core: &Core, lo: u32, hi: u32) -> TransReq {
        TransReq::capture(core, lo, hi).expect("a VISA range")
    }

    #[test]
    fn the_words_are_mounted_where_parcel_at_reads_them_and_nowhere_else() {
        let live = core_with(&program());
        let r = req(&live, CODE, CODE + 6);
        let mut s = Scratch::new(None);
        s.translate_with(probe, &UopConfig::default(), &r).unwrap();
        for k in 0..r.words.len() as u32 {
            assert_eq!(
                s.core.parcel_at(CODE + k),
                r.words[k as usize],
                "parcel {k}"
            );
        }
        let past = CODE + r.words.len() as u32;
        assert!((past..past + 64).all(|pc| s.core.parcel_at(pc) == 0));
        assert!((CODE - 64..CODE).all(|pc| s.core.parcel_at(pc) == 0));
        // a range at the top of the short-word space wraps like the byte index of `parcel_at`
        let top = 0x1f_fffe;
        let wrapped = TransReq {
            lo: top,
            hi: top + 2,
            words: vec![0x0001, 0x0001, 0x0001, 0x0001].into_boxed_slice(),
        };
        s.translate_with(probe, &UopConfig::default(), &wrapped)
            .unwrap();
        assert_eq!(
            (0..4)
                .map(|k| s.core.parcel_at(top + k))
                .collect::<Vec<_>>(),
            [1; 4]
        );
        assert_eq!(s.core.parcel_at(CODE), 0, "the first request was cleared");
    }

    #[test]
    fn the_scratch_answer_equals_the_answer_on_the_live_core_and_needs_the_tail() {
        let mut live = core_with(&program());
        let cfg = UopConfig::default();
        // the instruction at +5 starts inside [CODE, CODE + 6) and ends two parcels past it
        let (lo, hi) = (CODE, CODE + 6);
        let direct = probe(&mut live, &cfg, lo, hi).unwrap();
        let exercised = direct.0.len();
        assert_eq!(
            exercised, 4,
            "NOP, add, NOP, add: the probe ran on four instructions"
        );
        let want = render(&direct.0, &direct.1);
        let mut s = Scratch::new(None);
        let got = s.translate_with(probe, &cfg, &req(&live, lo, hi)).unwrap();
        assert_eq!(render(&got.blocks, &got.uops), want);
        // the same request without its tail decodes the last instruction from zero parcels, so
        // the comparison above is sensitive to what the scratch core holds
        let mut cut = req(&live, lo, hi);
        cut.words = cut.words[..(hi - lo) as usize].into();
        let short = s.translate_with(probe, &cfg, &cut).unwrap();
        assert_ne!(
            render(&short.blocks, &short.uops),
            want,
            "the tail words matter"
        );
    }

    #[test]
    fn a_request_sees_nothing_of_the_one_before_it() {
        let mut live = core_with(&program());
        let cfg = UopConfig::default();
        let mut s = Scratch::new(None);
        s.translate_with(probe, &cfg, &req(&live, CODE, CODE + 9))
            .unwrap();
        // the same addresses now hold other code: a stale decode entry or a stale parcel would
        // change the answer
        let other = [
            0x0001, 0x0001, 0x013e, 0x0000, 0x1312, 0x0001, 0x0001, 0x0001, 0x0001,
        ];
        for (k, p) in other.iter().enumerate() {
            crate::core::support::load(&mut live, CODE + k as u32, &[*p]);
        }
        let want = probe(&mut live, &cfg, CODE, CODE + 9).unwrap();
        let got = s
            .translate_with(probe, &cfg, &req(&live, CODE, CODE + 9))
            .unwrap();
        assert_eq!(render(&got.blocks, &got.uops), render(&want.0, &want.1));
        assert_eq!(got.blocks.len(), 7, "NOP, NOP, add, then four NOPs");
    }

    #[test]
    fn the_default_translator_is_translate_range() {
        // With the W0 stub both sides are empty; once `translate_range` lowers real blocks this
        // is the equality the worker contract promises (W3-FUZZ turns it into evidence).
        let mut live = core_with(&program());
        let cfg = UopConfig::default();
        let direct = translate_range(&mut live, &cfg, CODE, CODE + 9).unwrap();
        let got = Scratch::new(None)
            .translate(&cfg, &req(&live, CODE, CODE + 9))
            .unwrap();
        assert_eq!(render(&got.blocks, &got.uops), render(&direct.0, &direct.1));
    }

    #[test]
    fn a_range_the_words_do_not_cover_or_an_empty_one_is_out_of_range() {
        let live = core_with(&program());
        let cfg = UopConfig::default();
        let mut s = Scratch::new(None);
        let mut short = req(&live, CODE, CODE + 6);
        short.hi = CODE + 6 + 4 * super::super::TAIL_WORDS as u32;
        assert_eq!(
            s.translate(&cfg, &short).unwrap_err(),
            UopError::OutOfRange {
                lo: short.lo,
                hi: short.hi
            }
        );
        let mut empty = req(&live, CODE, CODE + 6);
        empty.hi = empty.lo;
        assert_eq!(
            s.translate(&cfg, &empty).unwrap_err(),
            UopError::OutOfRange { lo: CODE, hi: CODE }
        );
        empty.hi = empty.lo - 1;
        assert!(s.translate(&cfg, &empty).is_err(), "an inverted range");
        // a refused request leaves the scratch core usable
        let ok = s.translate_with(probe, &cfg, &req(&live, CODE, CODE + 6));
        assert_eq!(ok.unwrap().blocks.len(), 4);
    }

    #[test]
    fn the_stop_pc_is_set_on_the_scratch_core_and_can_change() {
        let mut s = Scratch::new(Some(CODE + 3));
        assert_eq!(s.core.uop_run.stop_pc, CODE + 3);
        s.set_stop_pc(None);
        assert_eq!(s.core.uop_run.stop_pc, NONE);
        assert!(
            !s.core.uop_enabled(),
            "a scratch core never arms the engine"
        );
    }
}
