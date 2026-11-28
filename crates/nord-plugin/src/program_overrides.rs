//! Who wins per block: the loaded program or the host parameter (D169).
//!
//! `Stage4Params::sync_to_engine` runs every block. Most host parameters have no program
//! counterpart that matters, so the host value simply wins (documented, TD-21). Two control groups have
//! one, and an untouched host default must not erase it:
//!
//! | Host parameter | Program state it would overwrite |
//! |---|---|
//! | `piano_touch` (one global) | `PianoLayer::touch` of Piano A and B (D99, D165) |
//! | `fxsel_mod2_type_n`, `fxsel_mod2_rate_n`, `fxy_mod2_var_n`, `fxx_mod2_amount_n` | the firmware Mod 2 raw words and Mod 2 settings of layer n (D163) |
//!
//! Rule: a tracked control **overrides** the program only after the host changed it from the
//! value it had when the program was loaded (or, before any load, from its default). Until
//! then the engine keeps what the program set. A later program load ends every override and takes
//! the current host values as the new baseline.
//!
//! Compatibility: a saved session restores its parameters before `initialize`; every restored value
//! that differs from the default counts as a host change, so an old session renders exactly as
//! before (the plugin had no program load path then, so there was never a program to preserve).
//! Undo and state recall are host changes too. With nothing loaded, an unarmed control leaves the
//! engine at its defaults, which equal the parameter defaults, so an untouched host is bit-identical.
//!
//! Audio-thread safe: relaxed atomics only, no allocation, no lock. `observe` runs on the audio
//! thread; `rebaseline` runs on the thread that owns the engine while it loads the program.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use nih_plug::prelude::Enum;

use crate::fx_params::FX_LAYERS;
use crate::params::Stage4Params;

/// Default of `fxsel_mod2_rate_n` in Hz.
///
/// ASSUMED: mirrors `LayerFxChain::new` (`mod2_rate_hz`), not a firmware value; confirmed only by
/// `fx_selectors::defaults_match_engine_defaults`, which fails if the engine default moves.
pub const MOD2_RATE_DEFAULT_HZ: f32 = 0.8;

/// Default of `fxx_mod2_amount_n`.
///
/// ASSUMED: mirrors the shipped host default (`unit("Mod 2 Amount", 0.35)` in `fx_extra_params`),
/// checked by `program_overrides::tests::default_params_arm_nothing`.
pub const MOD2_AMOUNT_DEFAULT: f32 = 0.35;

const TOUCH_BIT: u32 = 1;

fn mod2_bit(layer: usize) -> u32 {
    1 << (1 + layer)
}

/// Tracked per-layer Mod 2 values: type index, rate bits, variation flag, amount bits.
type Mod2Values = [u32; 4];

fn mod2_values(params: &Stage4Params, layer: usize) -> Mod2Values {
    let sel = &params.layer_fx_selectors.layers[layer];
    [
        sel.mod2_type.value().to_index() as u32,
        sel.mod2_rate_hz.value().to_bits(),
        u32::from(params.layer_fx_sharing.layers[layer].mod2_variation.value()),
        params.layer_fx_extra.layers[layer]
            .mod2_amount
            .value()
            .to_bits(),
    ]
}

/// Per-control "host overrides the program" flags plus the last host value seen.
pub struct ProgramOverrides {
    armed: std::sync::Arc<AtomicU32>,
    touch_last: AtomicU32,
    mod2_last: [[AtomicU32; 4]; FX_LAYERS],
}

impl Default for ProgramOverrides {
    /// Baseline = the parameter defaults (`Medium` Touch; Mod 2 Off, 0.8 Hz, variation off).
    fn default() -> Self {
        Self {
            armed: Default::default(),
            touch_last: AtomicU32::new(0),
            mod2_last: std::array::from_fn(|_| {
                [
                    AtomicU32::new(0),
                    AtomicU32::new(MOD2_RATE_DEFAULT_HZ.to_bits()),
                    AtomicU32::new(0),
                    AtomicU32::new(MOD2_AMOUNT_DEFAULT.to_bits()),
                ]
            }),
        }
    }
}

impl ProgramOverrides {
    /// Compares every tracked host value with the last one seen and arms the ones that moved.
    /// Call at the start of each `sync_to_engine`.
    pub fn observe(&self, params: &Stage4Params) {
        let touch = params.piano_touch.value() as u32;
        if touch != self.touch_last.swap(touch, Relaxed) {
            self.armed.fetch_or(TOUCH_BIT, Relaxed);
        }
        for (layer, slots) in self.mod2_last.iter().enumerate() {
            let now = mod2_values(params, layer);
            let mut moved = false;
            for (last, v) in slots.iter().zip(now) {
                moved |= last.swap(v, Relaxed) != v;
            }
            if moved {
                self.armed.fetch_or(mod2_bit(layer), Relaxed);
            }
        }
    }

    /// The armed flags, shared with the project state (`library_state.rs`).
    pub fn armed_handle(&self) -> std::sync::Arc<AtomicU32> {
        std::sync::Arc::clone(&self.armed)
    }

    /// Arms the flags in `mask` after a program load: a restored project's host values win (D261).
    pub fn arm(&self, mask: u32) {
        self.armed.fetch_or(mask, Relaxed);
    }

    /// The host `piano_touch` value replaces the program Touch of both Piano layers.
    pub fn touch_overrides(&self) -> bool {
        self.armed.load(Relaxed) & TOUCH_BIT != 0
    }

    /// The host Mod 2 values (type, rate, variation) of `layer` replace the program's.
    pub fn mod2_overrides(&self, layer: usize) -> bool {
        self.armed.load(Relaxed) & mod2_bit(layer) != 0
    }

    /// Every control overrides the program (test helper for the per-bank tests).
    #[cfg(test)]
    pub fn all_armed() -> Self {
        let o = Self::default();
        o.armed.store(u32::MAX, Relaxed);
        o
    }

    /// A program was just applied to the engine: the program wins again and the current host
    /// values become the baseline a later host change is measured against.
    pub fn rebaseline(&self, params: &Stage4Params) {
        self.armed.store(0, Relaxed);
        self.touch_last
            .store(params.piano_touch.value() as u32, Relaxed);
        for (layer, slots) in self.mod2_last.iter().enumerate() {
            for (last, v) in slots.iter().zip(mod2_values(params, layer)) {
                last.store(v, Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx_selectors::Mod2Sel;
    use nih_plug::prelude::{BoolParam, EnumParam, IntParam, IntRange};

    #[test]
    fn default_params_arm_nothing() {
        let p = Stage4Params::default();
        p.program_overrides.observe(&p);
        assert!(!p.program_overrides.touch_overrides());
        assert!((0..FX_LAYERS).all(|l| !p.program_overrides.mod2_overrides(l)));
    }

    #[test]
    fn a_moved_value_arms_only_its_control() {
        let mut p = Stage4Params {
            piano_touch: IntParam::new("t", 2, IntRange::Linear { min: 0, max: 2 }),
            ..Default::default()
        };
        p.layer_fx_selectors.layers[3].mod2_type = EnumParam::new("m", Mod2Sel::Chorus);
        p.layer_fx_sharing.layers[5].mod2_variation = BoolParam::new("v", true);
        let o = ProgramOverrides::default();
        o.observe(&p);
        assert!(o.touch_overrides());
        assert!(o.mod2_overrides(3) && o.mod2_overrides(5));
        assert!(!o.mod2_overrides(0) && !o.mod2_overrides(6));
    }

    #[test]
    fn rebaseline_clears_overrides_and_takes_current_values() {
        let mut p = Stage4Params {
            piano_touch: IntParam::new("t", 1, IntRange::Linear { min: 0, max: 2 }),
            ..Default::default()
        };
        let o = ProgramOverrides::default();
        o.observe(&p);
        assert!(o.touch_overrides());
        o.rebaseline(&p);
        assert!(!o.touch_overrides());
        o.observe(&p);
        assert!(
            !o.touch_overrides(),
            "the value did not move since the load"
        );
        p.piano_touch = IntParam::new("t", 2, IntRange::Linear { min: 0, max: 2 });
        o.observe(&p);
        assert!(o.touch_overrides(), "a move after the load wins again");
    }
}
