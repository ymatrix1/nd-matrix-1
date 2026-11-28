//! Re-strike of a note whose earlier voices the damper pedal still holds (opt-in, D263).
//!
//! Evidence: `docs/research/piano-pedal-keyup.md` s3 (`tools/arm_piano_keyup.py`, scenario `d`).
//! On the real firmware, CC64 = 127 then note-on, note-off, note-on of the same key: the second
//! note-on writes the hold word (`F14 = 1.0, F12 = F11 = 0, F16 = 1.0`, the same write the
//! note-off under the pedal makes) to both voice records of the first note and starts a fresh
//! pair on two other records. No fast-damp or release decay reaches the old pair, so it keeps its
//! hold decay until the pedal lifts. The engine damped the previous instance of a re-struck note
//! with its soft release whatever the pedal does; with `firmware_pedal_restrike` it leaves the
//! pedal-held instances ringing.
//!
//! Open (ARM harness limit, same note): the firmware key-up record and the release decay of the
//! held pair at the pedal lift were not observed, so the key-up count after a re-strike is the
//! engine's own rule: one key-up sample per note, from the loudest released voice (ASSUMED).

use super::PianoLayer;

/// One pending key-up at a pedal lift: `(note, velocity, amplitude, gain)`.
pub(super) type KeyUp = (u8, u8, f32, f32);

impl PianoLayer {
    /// `true` when a note-on must leave pedal-held (`sustained`) voices of the same note ringing
    /// instead of damping them (`firmware_pedal_restrike` with the damper pedal down).
    pub(super) fn restrike_keeps_held_voices(&self) -> bool {
        self.firmware_pedal_restrike && self.damper_pedal_down
    }
}

/// Adds `entry` to `list[..*count]`. With `one_per_note` an entry of a note already listed
/// replaces it only when it is louder, so a re-struck note gets one key-up sample. Without the
/// flag every voice is appended as before. Returns without writing when the list is full.
pub(super) fn push_key_up(list: &mut [KeyUp], count: &mut usize, entry: KeyUp, one_per_note: bool) {
    if one_per_note {
        if let Some(slot) = list[..*count].iter_mut().find(|e| e.0 == entry.0) {
            if entry.2 > slot.2 {
                *slot = entry;
            }
            return;
        }
    }
    if *count < list.len() {
        list[*count] = entry;
        *count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::build_test_wav_i16;
    use super::super::NpnoSound;
    use super::*;
    use std::sync::Arc;

    /// SFZ fixture with a normal and a key-up (`trigger=release`) region on note 60.
    fn key_up_sound() -> Arc<NpnoSound> {
        let sfz = "<group> lokey=c4 hikey=b4 pitch_keycenter=60
<region> sample=n.wav
<region> sample=r.wav trigger=release
";
        let wav = build_test_wav_i16(48_000, 261.63, 16_384);
        Arc::new(NpnoSound::from_sfz_str("Key Up", sfz, |_| Ok(wav.clone())).expect("SFZ parses"))
    }

    fn layer(restrike: bool) -> PianoLayer {
        let mut piano = PianoLayer::new(48_000.0);
        piano.set_sound(Some(key_up_sound()));
        piano.firmware_pedal_restrike = restrike;
        piano
    }

    fn held(piano: &PianoLayer) -> usize {
        piano
            .voices
            .iter()
            .filter(|v| v.active && !v.is_release_sample && v.sustained && !v.releasing)
            .count()
    }

    fn key_ups(piano: &PianoLayer) -> usize {
        piano
            .voices
            .iter()
            .filter(|v| v.active && v.is_release_sample)
            .count()
    }

    /// Pedal down, strike, key up, strike again: the state before the pedal lifts.
    fn restruck(restrike: bool) -> PianoLayer {
        let mut piano = layer(restrike);
        piano.set_damper_pedal(true, 64);
        piano.note_on(60, 90);
        piano.note_off(60);
        piano.note_on(60, 70);
        piano
    }

    #[test]
    fn the_flag_is_off_by_default() {
        assert!(!PianoLayer::new(48_000.0).firmware_pedal_restrike);
    }

    #[test]
    fn default_damps_the_older_instance_of_a_restruck_note() {
        let piano = restruck(false);
        let old: Vec<_> = piano.voices.iter().filter(|v| v.active).collect();
        assert_eq!(old.len(), 2, "old instance plus the new one");
        assert_eq!(old.iter().filter(|v| v.releasing).count(), 1);
        assert_eq!(
            held(&piano),
            0,
            "nothing is pedal-held yet: new voice is key-down"
        );
    }

    #[test]
    fn opt_in_keeps_the_pedal_held_instance_ringing() {
        let piano = restruck(true);
        assert_eq!(piano.voices.iter().filter(|v| v.active).count(), 2);
        assert_eq!(
            held(&piano),
            1,
            "the first instance stays held by the pedal"
        );
        assert!(piano
            .voices
            .iter()
            .filter(|v| v.active)
            .all(|v| !v.releasing));
    }

    #[test]
    fn opt_in_without_the_pedal_still_damps_a_doubled_note_on() {
        let mut piano = layer(true);
        piano.note_on(60, 90);
        piano.note_on(60, 70);
        assert_eq!(
            piano
                .voices
                .iter()
                .filter(|v| v.active && v.releasing)
                .count(),
            1
        );
    }

    #[test]
    fn opt_in_pedal_lift_releases_both_instances_with_one_key_up() {
        let mut piano = restruck(true);
        piano.note_off(60);
        assert_eq!(held(&piano), 2);
        assert_eq!(key_ups(&piano), 0, "no key-up under the pedal");
        piano.set_damper_pedal(false, 64);
        assert_eq!(held(&piano), 0);
        assert!(piano
            .voices
            .iter()
            .filter(|v| v.active && !v.is_release_sample)
            .all(|v| v.releasing));
        assert_eq!(key_ups(&piano), 1, "one key-up per note");
    }

    #[test]
    fn default_pedal_lift_spawns_a_key_up_per_released_voice() {
        // Legacy list: the first instance was damped at the re-strike, so one voice is held.
        let mut piano = restruck(false);
        piano.note_off(60);
        piano.set_damper_pedal(false, 64);
        assert!(key_ups(&piano) >= 1);
    }

    #[test]
    fn push_key_up_keeps_the_louder_entry_per_note() {
        let mut list = [(0, 0, 0.0, 0.0); 4];
        let mut n = 0;
        push_key_up(&mut list, &mut n, (60, 90, 0.2, 1.0), true);
        push_key_up(&mut list, &mut n, (60, 70, 0.5, 1.0), true);
        push_key_up(&mut list, &mut n, (60, 50, 0.1, 1.0), true);
        push_key_up(&mut list, &mut n, (62, 80, 0.3, 1.0), true);
        assert_eq!(n, 2);
        assert_eq!(list[0], (60, 70, 0.5, 1.0));
        assert_eq!(list[1], (62, 80, 0.3, 1.0));
    }

    #[test]
    fn push_key_up_without_the_flag_appends_and_stops_when_full() {
        let mut list = [(0, 0, 0.0, 0.0); 2];
        let mut n = 0;
        for amp in [0.1, 0.2, 0.3] {
            push_key_up(&mut list, &mut n, (60, 90, amp, 1.0), false);
        }
        assert_eq!(n, 2, "third entry dropped, list full");
        assert_eq!(list[1].2, 0.2);
    }
}
