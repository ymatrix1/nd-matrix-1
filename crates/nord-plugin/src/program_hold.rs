//! Who owns the engine per parameter group after a program load: the program or the host (D218).
//!
//! `Stage4Params::sync_to_engine` runs every block and writes every host parameter into the engine,
//! so a loaded `.ns4p` would be erased within one block (the layer enables, levels, drawbars,
//! effects ... all come back as the host defaults). D169 solved this for piano Touch and Mod 2 only.
//! This module generalises it to whole parameter groups:
//!
//! * A program (or firmware option) load **holds** the groups it set: the block sync skips them,
//!   so the engine keeps the program's values, including the ones no host parameter can express.
//! * Each block [`ProgramHold::observe`] compares every held parameter with the value it had at
//!   the load. A group whose parameter moved (the user touched a control, the host automated it, a
//!   session was restored) is **armed**: from then on the block sync writes that whole group from
//!   the host parameters again, exactly as before this module existed.
//! * A new load holds the groups again and takes the current host values as the new baseline.
//!
//! Before the first load nothing is held, so an untouched or restored session behaves exactly as
//! it did before (old sessions render bit-identically).
//!
//! Real-time safety: the parameter table is built once on the GUI / init thread ([`ProgramHold::bind`],
//! one allocation); after that `hold` / `observe` / `runs` are relaxed atomics over a fixed slice.

use crate::params::Stage4Params;
use nih_plug::prelude::{ParamPtr, Params};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};

/// A set of host parameters that the block sync writes together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Group {
    /// Layer enables, scene II switch and the split points.
    Layers,
    /// The seven layer levels, the master level (the engine level is their product) and the
    /// master compressor, which a program also stores.
    Levels,
    /// Organ model, drawbars, percussion, rotary (`organ_*`, `drawbar_*`, `rotary_*`).
    Organ,
    /// Piano category, timbre, string resonance, pedal noise (Touch has its own rule, D169).
    Piano,
    /// The shared synth oscillator mode, filter and envelope (`synth_*`).
    Synth,
    Morph,
    LayerFx,
    LayerFxSelectors,
    LayerFxExtra,
    LayerFxSharing,
    LayerFxVariation,
    PianoLayers,
    SynthLayers,
    SynthMod,
    SynthGate,
    SynthPattern,
    Pedals,
    PedalAssign,
    System,
    AuxKb,
    OrganLayers,
    OrganLive,
    OrganExtra,
    PianoExtra,
    MidiMenu,
    Output,
    MenuExtras,
    /// `fwx_*`: the firmware-law switches, also set by the firmware panel.
    Firmware,
    /// `prog_*`: transpose and program level.
    ProgramArea,
    /// Transport tempo, wheels and the MIDI routing mode: always the host's.
    Live,
}

impl Group {
    /// Groups a program load holds: everything except [`Group::Live`] and [`Group::Firmware`].
    ///
    /// The firmware switches are engine behaviour flags, not program data (`firmware_params.rs`):
    /// a program load leaves them alone, only the firmware panel holds them.
    pub const PROGRAM: u32 = ALL & !(1 << Group::Live as u32) & !(1 << Group::Firmware as u32);
    /// Groups a firmware-options load holds.
    pub const FIRMWARE: u32 = 1 << Group::Firmware as u32;

    const fn bit(self) -> u32 {
        1 << self as u32
    }
}

const ALL: u32 = (1 << (Group::Live as u32 + 1)) - 1;

/// Group of a host parameter id (the part before the first `_` names the family).
///
/// Every id of `Stage4Params` must map to something other than `None`;
/// `tests::every_host_parameter_belongs_to_a_group` fails for a family added without a choice.
pub fn group_of(id: &str) -> Option<Group> {
    let family = id.split('_').next()?;
    let tail = |suffix: &str| id.ends_with(suffix);
    Some(match family {
        "master" if id == "master_level" || id == "master_compressor" => Group::Levels,
        // `library_program` picks the program itself: a load must not re-arm anything.
        "master" | "midi" | "mod" | "control" | "aftertouch" | "library" => Group::Live,
        "split" | "layer" => Group::Layers,
        "organ" | "piano" | "synth" if tail("_enabled") => Group::Layers,
        "organ" | "piano" | "synth" if tail("_level") => Group::Levels,
        "organ" | "drawbar" | "rotary" => Group::Organ,
        "piano" => Group::Piano,
        "synth" => Group::Synth,
        "morph" => Group::Morph,
        "fx" => Group::LayerFx,
        "fxsel" => Group::LayerFxSelectors,
        "fxx" => Group::LayerFxExtra,
        "fxy" => Group::LayerFxSharing,
        "fxz" => Group::LayerFxVariation,
        "pianox" => Group::PianoLayers,
        "synthx" => Group::SynthLayers,
        "synthm" => Group::SynthMod,
        "synthg" => Group::SynthGate,
        "synthp" => Group::SynthPattern,
        "ped" => Group::Pedals,
        "pedx" => Group::PedalAssign,
        "sys" => Group::System,
        "auxkb" => Group::AuxKb,
        "organx" => Group::OrganLayers,
        "organz" => Group::OrganLive,
        "organy" => Group::OrganExtra,
        "pianoy" => Group::PianoExtra,
        "midix" => Group::MidiMenu,
        "outx" => Group::Output,
        "menx" => Group::MenuExtras,
        "fwx" => Group::Firmware,
        "prog" => Group::ProgramArea,
        _ => return None,
    })
}

/// One tracked host parameter.
struct Entry {
    #[cfg(test)]
    id: String,
    ptr: ParamPtr,
    group: Group,
    /// Normalized value (bits) at the last load of its group.
    baseline: AtomicU32,
}

// SAFETY: `ptr` points into the `Stage4Params` that owns this table (see `ProgramHold::bind`), is
// only read through atomic loads (`unmodulated_normalized_value`), and the table is dropped with
// the params, so it never outlives its targets.
unsafe impl Send for Entry {}
unsafe impl Sync for Entry {}

/// Per-group program ownership, see the module docs.
#[derive(Default)]
pub struct ProgramHold {
    /// Groups the program owns (bit per [`Group`]).
    held: AtomicU32,
    /// Held groups whose parameters moved since the load: the host writes them again.
    armed: Arc<AtomicU32>,
    table: OnceLock<Box<[Entry]>>,
    /// Host values the tests set: the NIH-plug parameter setters are crate-private.
    #[cfg(test)]
    test_values: std::sync::Mutex<std::collections::HashMap<String, f32>>,
}

impl ProgramHold {
    /// Builds the parameter table. Call once, after `params` reached its final address (inside
    /// its `Arc`) and off the audio thread. Before this, [`Self::hold`] holds nothing, so a
    /// `Stage4Params` used on its own (unit tests) syncs everything as before.
    pub fn bind(&self, params: &Stage4Params) {
        let entries: Box<[Entry]> = params
            .param_map()
            .into_iter()
            .filter_map(|(id, ptr, _)| {
                let group = group_of(&id)?;
                Some(Entry {
                    #[cfg(test)]
                    id: id.clone(),
                    ptr,
                    group,
                    // SAFETY: `ptr` was just taken from `params`, which is alive and borrowed.
                    baseline: AtomicU32::new(
                        unsafe { ptr.unmodulated_normalized_value() }.to_bits(),
                    ),
                })
            })
            .collect();
        let _ = self.table.set(entries);
    }

    /// Current normalized value of a tracked parameter.
    fn read(&self, e: &Entry) -> f32 {
        #[cfg(test)]
        if let Some(v) = self.test_values.lock().unwrap().get(&e.id) {
            return *v;
        }
        // SAFETY: see `Entry`.
        unsafe { e.ptr.unmodulated_normalized_value() }
    }

    /// Pretends the host moved parameter `id` to `normalized` (tests only).
    #[cfg(test)]
    pub(crate) fn set_test_value(&self, id: &str, normalized: f32) {
        self.test_values
            .lock()
            .unwrap()
            .insert(id.into(), normalized);
    }

    /// `true` once [`Self::bind`] ran.
    pub fn is_bound(&self) -> bool {
        self.table.get().is_some()
    }

    /// The program (or firmware options) just set the groups in `mask`: hold them, and measure
    /// later host moves against the values the parameters have now. No effect when unbound.
    pub fn hold(&self, mask: u32) {
        let Some(table) = self.table.get() else {
            return;
        };
        for e in table.iter().filter(|e| mask & e.group.bit() != 0) {
            e.baseline.store(self.read(e).to_bits(), Relaxed);
        }
        self.armed.fetch_and(!mask, Relaxed);
        self.held.fetch_or(mask, Relaxed);
    }

    /// Arms every held group with a parameter that moved since its load. Call once per block
    /// before the sync.
    pub fn observe(&self) {
        let held = self.held.load(Relaxed);
        let Some(table) = self.table.get().filter(|_| held != 0) else {
            return;
        };
        let mut armed = self.armed.load(Relaxed);
        for e in table {
            let bit = e.group.bit();
            if held & bit == 0 || armed & bit != 0 {
                continue;
            }
            if self.read(e).to_bits() != e.baseline.load(Relaxed) {
                armed |= bit;
            }
        }
        self.armed.store(armed, Relaxed);
    }

    /// The block sync writes `group` from the host parameters (not held, or armed).
    pub fn runs(&self, group: Group) -> bool {
        let bit = group.bit();
        self.held.load(Relaxed) & bit == 0 || self.armed.load(Relaxed) & bit != 0
    }

    /// The program owns `group` right now (held and not armed): the panel shows the program's
    /// values for it instead of the host parameters.
    pub fn owned_by_program(&self, group: Group) -> bool {
        !self.runs(group)
    }

    /// The armed mask, shared with the project state (`library_state.rs`) so a save sees the edits.
    pub fn armed_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.armed)
    }

    /// Arms the held groups in `mask` again: a restored project's edits win over the program the
    /// library just loaded (D261). Call after [`Self::hold`].
    pub fn arm(&self, mask: u32) {
        self.armed.fetch_or(mask & self.held.load(Relaxed), Relaxed);
    }

    /// Releases the groups in `mask`: the host parameters drive them again from the next sync.
    pub fn release(&self, mask: u32) {
        self.held.fetch_and(!mask, Relaxed);
        self.armed.fetch_and(!mask, Relaxed);
    }

    /// Releases every group (tests, and the plugin when the program is cleared).
    pub fn release_all(&self) {
        self.held.store(0, Relaxed);
        self.armed.store(0, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn bound() -> Arc<Stage4Params> {
        let p = Arc::new(Stage4Params::default());
        p.program_hold.bind(&p);
        p
    }

    #[test]
    fn every_host_parameter_belongs_to_a_group() {
        let p = Stage4Params::default();
        let unmapped: Vec<_> = p
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .filter(|id| group_of(id).is_none())
            .collect();
        assert!(unmapped.is_empty(), "no hold group for {unmapped:?}");
    }

    #[test]
    fn top_level_families_map_as_documented() {
        let g = |id| group_of(id).unwrap();
        assert_eq!(g("master_level"), Group::Levels);
        assert_eq!(g("master_compressor"), Group::Levels);
        assert_eq!(g("master_bpm"), Group::Live);
        assert_eq!(g("mod_wheel"), Group::Live);
        assert_eq!(g("organ_a_enabled"), Group::Layers);
        assert_eq!(g("synth_c_level"), Group::Levels);
        assert_eq!(g("drawbar_b_9_1"), Group::Organ);
        assert_eq!(g("piano_category"), Group::Piano);
        assert_eq!(g("synth_attack"), Group::Synth);
        assert_eq!(g("synthx_glide_1"), Group::SynthLayers);
        assert_eq!(g("fwx_arp_walk"), Group::Firmware);
        assert_eq!(g("fx_drive_3"), Group::LayerFx);
        assert_eq!(group_of("unknown_thing"), None);
    }

    #[test]
    fn a_program_load_holds_everything_but_live_and_firmware() {
        assert_eq!(Group::PROGRAM & Group::Live.bit(), 0);
        assert_eq!(Group::PROGRAM & Group::Firmware.bit(), 0);
        assert_ne!(Group::PROGRAM & Group::Organ.bit(), 0);
        assert_ne!(Group::PROGRAM & Group::LayerFx.bit(), 0);
    }

    #[test]
    fn arm_after_a_load_restores_the_hosts_edits_but_only_for_held_groups() {
        let p = bound();
        p.program_hold.hold(Group::PROGRAM);
        assert!(p.program_hold.owned_by_program(Group::Synth));
        p.program_hold.arm(Group::Synth.bit() | Group::Live.bit());
        assert!(p.program_hold.runs(Group::Synth), "the edit wins again");
        assert!(
            p.program_hold.owned_by_program(Group::Organ),
            "others stay the program's"
        );
        assert_eq!(
            p.program_hold.armed_handle().load(Relaxed),
            Group::Synth.bit()
        );
    }

    #[test]
    fn nothing_is_held_before_a_load_and_an_unbound_hold_holds_nothing() {
        let p = bound();
        assert_eq!(Group::PROGRAM & !ALL, 0);
        assert!(p.program_hold.runs(Group::Organ));
        let unbound = ProgramHold::default();
        unbound.hold(Group::PROGRAM);
        assert!(unbound.runs(Group::Organ), "no table: nothing can be held");
    }

    #[test]
    fn a_load_holds_the_groups_and_a_moved_parameter_arms_only_its_group() {
        let p = bound();
        let h = &p.program_hold;
        h.hold(Group::PROGRAM);
        h.observe();
        assert!(h.owned_by_program(Group::Organ) && h.owned_by_program(Group::LayerFx));
        assert!(h.runs(Group::Live), "the transport is never held");

        // A host move of a drawbar: the Organ group is the host's again, the rest stays held.
        h.set_test_value("drawbar_1_16", 0.0);
        h.observe();
        assert!(h.runs(Group::Organ));
        assert!(h.owned_by_program(Group::Synth) && h.owned_by_program(Group::Levels));
    }

    #[test]
    fn a_new_load_holds_again_and_takes_the_current_values_as_baseline() {
        let p = bound();
        let h = &p.program_hold;
        h.hold(Group::PROGRAM);
        h.set_test_value("synth_attack", 0.9);
        h.observe();
        assert!(h.runs(Group::Synth));
        h.hold(Group::PROGRAM);
        h.observe();
        assert!(
            h.owned_by_program(Group::Synth),
            "the new baseline is the moved value"
        );
    }

    #[test]
    fn a_firmware_hold_leaves_the_program_groups_alone() {
        let p = bound();
        let h = &p.program_hold;
        h.hold(Group::FIRMWARE);
        assert!(h.owned_by_program(Group::Firmware));
        assert!(h.runs(Group::Organ));
        h.release_all();
        assert!(h.runs(Group::Firmware));
    }

    #[test]
    fn releasing_the_program_groups_keeps_the_firmware_hold() {
        let p = bound();
        let h = &p.program_hold;
        h.hold(Group::PROGRAM | Group::FIRMWARE);
        assert!(h.owned_by_program(Group::Synth) && h.owned_by_program(Group::Firmware));
        h.release(Group::PROGRAM);
        assert!(
            h.runs(Group::Synth),
            "program groups run from the host again"
        );
        assert!(h.owned_by_program(Group::Firmware), "firmware stays held");
    }
}
