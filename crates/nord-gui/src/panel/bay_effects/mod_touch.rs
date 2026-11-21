//! Which engine edits a Mod 1 / Mod 2 control may send.
//!
//! The unit's full edit set (type, rate, amount, variation, ...) is a snapshot of the panel's
//! view. Sending all of it on every touch overwrites state the view does not know (a loaded
//! program's variation) and the Mod 2 type, rate and variation edits clear the firmware raw
//! words (`mod2_type_raw` / `mod2_rate_raw`), swapping the firmware stage for the engine one: an
//! audible jump on the first touch of an unrelated knob. [`Touch`] records what the user
//! changed this frame and [`keep`] filters the set down to it.

use crate::audio::{FxEdit, FxFlag, FxFloat};

/// What changed in one frame, as a bit set.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct Touch(u8);

impl Touch {
    /// ON button or type selector (the type edit; Mod 1 also its PED / MST CLK validity).
    pub const TYPE: Touch = Touch(1);
    /// Shift + selector toggling VARIATION.
    pub const VARIATION: Touch = Touch(2);
    /// Shift + selector toggling PED (Mod 1 Wah / Pump).
    pub const PED: Touch = Touch(4);
    /// RATE knob in PED mode (static position).
    pub const PED_POS: Touch = Touch(8);
    /// RATE knob in MST CLK mode (division).
    pub const MST: Touch = Touch(16);
    /// RATE knob (Hz or SENS).
    pub const RATE: Touch = Touch(32);
    /// AMOUNT knob.
    pub const AMOUNT: Touch = Touch(64);

    pub fn add(&mut self, other: Touch) {
        self.0 |= other.0;
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn has(self, other: Touch) -> bool {
        self.0 & other.0 != 0
    }
}

/// `true` when `e` belongs to a control in `touch`; [`FxEdit::Sharing`] always passes (it is the
/// GROUP / GLOBAL routing the edit itself may change).
pub(super) fn keep(touch: Touch, e: &FxEdit) -> bool {
    match e {
        FxEdit::Sharing(_) => true,
        FxEdit::Mod1Type { .. } | FxEdit::Mod2Type { .. } => touch.has(Touch::TYPE),
        // A new Mod 1 type changes whether PED and MST CLK apply.
        FxEdit::Mod1Sync { .. } => touch.has(Touch::TYPE) || touch.has(Touch::MST),
        FxEdit::Flag { param, .. } => match param {
            FxFlag::Mod1Variation | FxFlag::Mod2Variation => touch.has(Touch::VARIATION),
            FxFlag::Mod1Ped => touch.has(Touch::PED) || touch.has(Touch::TYPE),
            _ => false,
        },
        FxEdit::Float { param, .. } => match param {
            FxFloat::Mod1Rate | FxFloat::Mod2Rate => touch.has(Touch::RATE),
            FxFloat::Mod1PedPos => touch.has(Touch::PED_POS),
            FxFloat::Mod1Amount | FxFloat::Mod2Amount => touch.has(Touch::AMOUNT),
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_core::fx::{Mod1Type, Mod2Type};

    fn float(param: FxFloat) -> FxEdit {
        FxEdit::Float {
            slot: 0,
            param,
            value: 0.5,
        }
    }

    fn full_set() -> Vec<FxEdit> {
        vec![
            FxEdit::Mod1Type {
                slot: 0,
                kind: Mod1Type::AutoPan,
            },
            float(FxFloat::Mod1Rate),
            float(FxFloat::Mod1Amount),
            float(FxFloat::Mod1PedPos),
            FxEdit::Flag {
                slot: 0,
                param: FxFlag::Mod1Variation,
                on: false,
            },
            FxEdit::Flag {
                slot: 0,
                param: FxFlag::Mod1Ped,
                on: false,
            },
            FxEdit::Mod1Sync {
                slot: 0,
                beats: None,
            },
            FxEdit::Mod2Type {
                slot: 0,
                kind: Mod2Type::Chorus,
            },
            float(FxFloat::Mod2Rate),
            float(FxFloat::Mod2Amount),
            FxEdit::Flag {
                slot: 0,
                param: FxFlag::Mod2Variation,
                on: false,
            },
            float(FxFloat::Drive),
        ]
    }

    fn kept(t: Touch) -> Vec<FxEdit> {
        full_set().into_iter().filter(|e| keep(t, e)).collect()
    }

    #[test]
    fn every_control_keeps_only_its_own_edits() {
        assert_eq!(kept(Touch::AMOUNT).len(), 2, "both amounts, no other");
        assert_eq!(kept(Touch::RATE).len(), 2);
        assert_eq!(kept(Touch::PED_POS), vec![float(FxFloat::Mod1PedPos)]);
        assert_eq!(kept(Touch::VARIATION).len(), 2);
        assert_eq!(kept(Touch::MST).len(), 1);
        assert_eq!(kept(Touch::PED).len(), 1);
        // The type keeps both type edits, Mod 1 PED validity and its clock; never rate / amount.
        let t = kept(Touch::TYPE);
        assert_eq!(t.len(), 4, "{t:?}");
        assert!(t.iter().all(|e| !matches!(e, FxEdit::Float { .. })));
    }

    #[test]
    fn rate_and_amount_never_send_a_type_or_variation() {
        for t in [Touch::RATE, Touch::AMOUNT, Touch::PED_POS, Touch::MST] {
            assert!(kept(t)
                .iter()
                .all(|e| !matches!(e, FxEdit::Mod1Type { .. } | FxEdit::Mod2Type { .. })));
            assert!(kept(t).iter().all(|e| !matches!(
                e,
                FxEdit::Flag {
                    param: FxFlag::Mod2Variation | FxFlag::Mod1Variation,
                    ..
                }
            )));
        }
    }

    #[test]
    fn untouched_keeps_nothing_but_sharing() {
        assert!(kept(Touch::default()).is_empty());
        assert!(Touch::default().is_empty());
        let s = FxEdit::Sharing(Default::default());
        assert!(keep(Touch::default(), &s));
    }
}
