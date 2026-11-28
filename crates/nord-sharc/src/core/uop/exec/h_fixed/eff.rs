//! The effect of one fixed-point compute on one element, computed first and applied after.
//!
//! OWNER: W2-FIXED. A handler reads PEx and PEy through a [`View`], turns each into an [`Eff`]
//! without writing anything, and only then [`commit`]s. That order is the rule "precondition
//! before mutate" of the exit protocol (design 4.5 rule 1): when an effect would raise an
//! arithmetic-exception interrupt (`Eff::sticky != 0`) the handler returns `Exit::Slow` and the
//! core is exactly as it was, so `Core::step` runs the instruction and does the raise, the
//! `irq.due` poke and the sequencer prologue of the next step itself. Raising inside the block
//! would leave `irq.due` poked and skip that prologue for the following uops.
//!
//! What an effect can change, per element, is everything `Core::exec_compute_op` writes for a
//! fixed-point operation: up to three data registers (the float extension byte is dropped for ALU
//! and multiplier results and kept for shifter results and MR reads, exactly as the interpreter
//! does), the six ALU flags or the three shifter flags, ASTAT extra bits (AF, SF, MN..MI, CACC),
//! one multiplier result register (80 bits, kept sign extended) and the sticky exception bits.
use crate::core::types::Flags;
use crate::core::Core;

/// ASTAT extra bit 10 (AF): the last ALU operation was floating point. A fixed ALU operation
/// clears it. VERIFIED: `alu.rs::exec_compute_op` (`af = is_float_alu << 10`), checked by every
/// ALU case of `diff_ops.rs`.
pub(super) const AF: u32 = 1 << 10;
/// ASTAT extra bit 14 (SF), cleared by every shifter operation. VERIFIED: `shifter.rs::shifter_op`.
pub(super) const SF: u32 = 1 << 14;
/// ASTAT extra bits 6..9: MN, MV, MU, MI. VERIFIED: `multiplier.rs::set_mult_flags`.
pub(super) const MULT_FLAGS: u32 = 0xF << 6;
/// STKY bit AOS: fixed-point overflow. VERIFIED: `fixed_alu.rs::STKY_AOS`.
pub(super) const STKY_AOS: u32 = 1 << 2;
/// STKY bit MOS: fixed-point multiplier overflow. VERIFIED: `multiplier.rs::STKY_MOS`.
pub(super) const STKY_MOS: u32 = 1 << 6;
/// STKY bit MVS, set by `RND MRx` and `SAT MRx` overflow. VERIFIED: `multiplier.rs::STKY_MVS`.
pub(super) const STKY_MVS: u32 = 1 << 7;
/// STKY bit MUS. VERIFIED: `multiplier.rs::STKY_MUS`.
pub(super) const STKY_MUS: u32 = 1 << 8;

/// One data register write.
#[derive(Clone, Copy, Default)]
pub(super) struct Wr {
    pub n: u8,
    pub v: u32,
    /// Drops the float extension byte of the register (`ExtFile::clear`: the tag stays).
    pub clear_lo: bool,
}

/// The ALU flags a fixed result sets. AI is always cleared by a fixed ALU operation (VERIFIED:
/// `fixed_alu.rs`, census diff), so it is not a field.
#[derive(Clone, Copy, Default)]
pub(super) struct AluFlags {
    pub az: bool,
    pub an: bool,
    pub av: bool,
    pub ac: bool,
    pub as_: bool,
}

impl AluFlags {
    /// The flags of the result `r` with carry `ac`, overflow `av` and sign flag `as_`
    /// (`Core::fixed_result`).
    #[inline(always)]
    pub fn of(r: u32, ac: bool, av: bool, as_: bool) -> AluFlags {
        AluFlags {
            az: r == 0,
            an: (r as i32) < 0,
            av,
            ac,
            as_,
        }
    }

    /// Either of two flag sets (`Core::or_flags`, the flags of a dual add/subtract).
    #[inline(always)]
    pub fn or(self, o: AluFlags) -> AluFlags {
        AluFlags {
            az: self.az || o.az,
            an: self.an || o.an,
            av: self.av || o.av,
            ac: self.ac || o.ac,
            as_: self.as_ || o.as_,
        }
    }
}

/// What one fixed-point compute does to one element.
#[derive(Clone, Copy, Default)]
pub(super) struct Eff {
    /// Register writes in order; a later write to the same register wins.
    pub wr: [Wr; 3],
    pub nw: u8,
    /// New ALU flags (and AI cleared).
    pub alu: Option<AluFlags>,
    /// New shifter flags SZ, SV, SS.
    pub shf: Option<(bool, bool, bool)>,
    /// ASTAT extra bits to clear and then set.
    pub astat_clear: u32,
    pub astat_set: u32,
    /// A compare shifts `Rx > Ry` into CACC (ASTAT bits 31..24).
    pub cacc: Option<bool>,
    /// Multiplier result register (0 = MRF, 1 = MRB) and its new 80-bit value.
    pub mr: Option<(u8, i128)>,
    /// Sticky exception bits the operation raises (zero means it raises nothing).
    pub sticky: u32,
}

impl Eff {
    /// Nothing happens (the condition did not hold).
    pub const NONE: Eff = Eff {
        wr: [Wr {
            n: 0,
            v: 0,
            clear_lo: false,
        }; 3],
        nw: 0,
        alu: None,
        shf: None,
        astat_clear: 0,
        astat_set: 0,
        cacc: None,
        mr: None,
        sticky: 0,
    };

    /// Appends a register write.
    #[inline(always)]
    pub fn write(&mut self, n: u8, v: u32, clear_lo: bool) {
        // at most three writes exist (multiplier, sum, difference); the guard keeps the index
        // in range whatever a caller does
        if let Some(slot) = self.wr.get_mut(usize::from(self.nw)) {
            *slot = Wr { n, v, clear_lo };
            self.nw += 1;
        }
    }

    /// Appends the writes of `o` after the ones of `self` (so `o` wins on a shared register) and
    /// ORs the sticky bits. Flags, ASTAT bits and MR are taken by the caller.
    #[inline(always)]
    pub fn then_writes(&mut self, o: &Eff) {
        for w in &o.wr[..usize::from(o.nw)] {
            self.write(w.n, w.v, w.clear_lo);
        }
        self.sticky |= o.sticky;
    }
}

/// A read-only view of one element: what an operation may read.
pub(super) struct View<'a> {
    pub r: &'a [u32; 16],
    pub f: &'a Flags,
    pub mr: &'a [i128; 2],
    /// MODE1 ALUSAT.
    pub sat: bool,
}

/// The view of PEx (`y` false) or PEy of `core`.
#[inline(always)]
pub(super) fn view(core: &Core, y: bool) -> View<'_> {
    let sat = core.mode1 & crate::core::float::MODE1_ALUSAT != 0;
    if y {
        View {
            r: &core.s,
            f: &core.flags_y,
            mr: &core.mr_y,
            sat,
        }
    } else {
        View {
            r: &core.r,
            f: &core.flags,
            mr: &core.mr,
            sat,
        }
    }
}

/// Applies `e` to PEx (`y` false) or PEy of `core`. Never raises an interrupt: the caller bailed
/// out before when `e.sticky` was not zero.
#[inline(always)]
pub(super) fn commit(core: &mut Core, y: bool, e: &Eff) {
    let (r, lo, f, astat, mr) = if y {
        (
            &mut core.s,
            &mut core.fx_y.lo,
            &mut core.flags_y,
            &mut core.sys.astat_extra[1],
            &mut core.mr_y,
        )
    } else {
        (
            &mut core.r,
            &mut core.fx.lo,
            &mut core.flags,
            &mut core.sys.astat_extra[0],
            &mut core.mr,
        )
    };
    for w in &e.wr[..usize::from(e.nw)] {
        let n = usize::from(w.n & 15);
        r[n] = w.v;
        if w.clear_lo {
            lo[n] = 0;
        }
    }
    if let Some(a) = e.alu {
        (f.az, f.an, f.av, f.ac, f.as_, f.ai) = (a.az, a.an, a.av, a.ac, a.as_, false);
    }
    if let Some((sz, sv, ss)) = e.shf {
        (f.sz, f.sv, f.ss) = (sz, sv, ss);
    }
    *astat = *astat & !e.astat_clear | e.astat_set;
    if let Some(greater) = e.cacc {
        *astat = *astat & 0x00FF_FFFF | ((*astat >> 24 >> 1) | u32::from(greater) << 7) << 24;
    }
    if let Some((k, v)) = e.mr {
        mr[usize::from(k & 1)] = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    #[test]
    fn a_commit_writes_registers_flags_astat_and_mr_of_the_chosen_element_only() {
        for y in [false, true] {
            let mut c = core_with(&[0x0001]);
            c.fx.lo[3] = 7;
            c.fx_y.lo[3] = 9;
            c.sys.astat_extra = [AF | SF, AF | SF];
            let mut e = Eff::NONE;
            e.write(3, 0xAB, true);
            e.write(4, 0xCD, false);
            e.alu = Some(AluFlags::of(0, true, false, true));
            e.shf = Some((true, false, true));
            e.astat_clear = AF;
            e.astat_set = 1 << 6;
            e.cacc = Some(true);
            e.mr = Some((1, -5));
            commit(&mut c, y, &e);
            let (r, lo, f, astat, mr, other_r, other_lo, other_astat, other_mr) = if y {
                (
                    c.s,
                    c.fx_y.lo,
                    c.flags_y,
                    c.sys.astat_extra[1],
                    c.mr_y,
                    c.r,
                    c.fx.lo,
                    c.sys.astat_extra[0],
                    c.mr,
                )
            } else {
                (
                    c.r,
                    c.fx.lo,
                    c.flags,
                    c.sys.astat_extra[0],
                    c.mr,
                    c.s,
                    c.fx_y.lo,
                    c.sys.astat_extra[1],
                    c.mr_y,
                )
            };
            assert_eq!((r[3], r[4], lo[3]), (0xAB, 0xCD, 0), "y {y}");
            assert_eq!((f.az, f.an, f.ac, f.as_), (true, false, true, true));
            assert_eq!((f.sz, f.sv, f.ss), (true, false, true));
            assert_eq!(
                astat,
                SF | 1 << 6 | 0x80 << 24,
                "AF cleared, CACC shifted in"
            );
            assert_eq!(mr, [0, -5]);
            assert_eq!(
                (other_r, other_lo[3], other_astat, other_mr),
                ([0; 16], if y { 7 } else { 9 }, AF | SF, [0, 0]),
                "the other element is untouched"
            );
        }
    }

    #[test]
    fn a_later_write_to_the_same_register_wins_and_a_fourth_write_is_ignored() {
        let mut c = core_with(&[0x0001]);
        let mut e = Eff::NONE;
        for (n, v) in [(2, 1), (2, 2), (5, 3), (6, 4)] {
            e.write(n, v, true);
        }
        assert_eq!(e.nw, 3);
        commit(&mut c, false, &e);
        assert_eq!((c.r[2], c.r[5], c.r[6]), (2, 3, 0));
        let mut sum = Eff::NONE;
        sum.write(1, 10, true);
        sum.sticky = STKY_AOS;
        let mut all = Eff::NONE;
        all.write(0, 5, true);
        all.then_writes(&sum);
        assert_eq!((all.nw, all.sticky, all.wr[1].v), (2, STKY_AOS, 10));
    }

    #[test]
    fn flags_are_derived_from_the_result_and_or_together() {
        let a = AluFlags::of(0, true, false, false);
        let b = AluFlags::of(0x8000_0000, false, true, true);
        assert!(a.az && !a.an && a.ac);
        assert!(!b.az && b.an && b.av && b.as_);
        let o = a.or(b);
        assert!(o.az && o.an && o.av && o.ac && o.as_);
    }

    #[test]
    fn the_view_follows_the_element_and_alusat() {
        let mut c = core_with(&[0x0001]);
        c.r[1] = 11;
        c.s[1] = 22;
        c.mr = [1, 2];
        c.mr_y = [3, 4];
        c.flags.az = true;
        c.mode1 = crate::core::float::MODE1_ALUSAT;
        let (x, y) = (view(&c, false), view(&c, true));
        assert_eq!((x.r[1], y.r[1], *x.mr, *y.mr), (11, 22, [1, 2], [3, 4]));
        assert!(x.f.az && !y.f.az && x.sat && y.sat);
    }
}
