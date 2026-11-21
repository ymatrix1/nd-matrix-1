//! Piano voice-list census: the firmware list of a note-on (extra voice, main voice) against the
//! voices the engine starts with the damper pedal up, down (default legacy path) and down with
//! `firmware_pedal_voices` (opt-in).
//!
//! The firmware list is `NpnoLevel::voices` (VERIFIED against the ARM level calls, 96 of 96
//! notes, `docs/research/piano-sound-load.md` s10); with CC64 = 127 the firmware runs the same
//! level calls on 38 of 38 factory sounds (`docs/research/piano-extra-voices.md` s2). The census
//! asks which engine path reproduces that list. Derived numbers only; no audio.

use nord_core::piano::{NpnoSound, PianoLayer};
use std::sync::Arc;

const FS: f32 = 48_000.0;
const Q23_ONE: f32 = 8_388_608.0;
const TOL: f32 = 1e-6;

/// Notes swept (every third key of the 88-key range).
pub const NOTES: [u8; 30] = [
    21, 24, 27, 30, 33, 36, 39, 42, 45, 48, 51, 54, 57, 60, 63, 66, 69, 72, 75, 78, 81, 84, 87, 90,
    93, 96, 99, 102, 105, 108,
];
/// Velocities swept.
pub const VELOCITIES: [u8; 6] = [1, 32, 64, 90, 110, 127];

/// Counts of one sound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoundRow {
    pub name: String,
    pub class: u8,
    /// (note, velocity) points with a firmware voice list.
    pub points: usize,
    /// Points whose firmware list has two voices.
    pub two_voice: usize,
    /// Points where the extra voice is a damper-down stroke (the root has layer-1 strokes).
    pub damper_extra: usize,
    /// Engine pedal up equals the firmware list.
    pub up_eq: usize,
    /// Engine pedal down, default path, equals the firmware list.
    pub down_default_eq: usize,
    /// Engine pedal down with `firmware_pedal_voices` equals the firmware list.
    pub down_optin_eq: usize,
}

type List = Vec<(usize, f32)>;

fn firmware_list(sound: &NpnoSound, note: u8, velocity: u8) -> List {
    let Some(model) = sound.firmware_level.as_ref() else {
        return Vec::new();
    };
    model
        .voices(note, velocity)
        .iter()
        .map(|v| {
            (
                usize::from(v.record),
                (v.amplitude_q23 as f32 / Q23_ONE).min(1.0),
            )
        })
        .collect()
}

fn engine_list(sound: &Arc<NpnoSound>, pedal: bool, optin: bool, note: u8, vel: u8) -> List {
    let mut layer = PianoLayer::new(FS);
    layer.set_sound(Some(sound.clone()));
    layer.firmware_pedal_voices = optin;
    layer.set_damper_pedal(pedal, 64);
    layer.note_on(note, vel);
    layer
        .active_voice_strokes()
        .into_iter()
        .map(|(idx, _, amp)| (idx.unwrap_or(usize::MAX), amp))
        .collect()
}

fn same(a: &List, b: &List) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.0 == y.0 && (x.1 - y.1).abs() <= TOL)
}

/// Census of one loaded sound over [`NOTES`] x [`VELOCITIES`]; `None` without a level model.
pub fn census(name: &str, sound: &Arc<NpnoSound>) -> Option<SoundRow> {
    let model = sound.firmware_level.as_ref()?;
    let mut row = SoundRow {
        name: name.to_string(),
        class: model.class(),
        ..SoundRow::default()
    };
    for &note in &NOTES {
        for &vel in &VELOCITIES {
            let fw = firmware_list(sound, note, vel);
            if fw.is_empty() {
                continue;
            }
            row.points += 1;
            row.two_voice += usize::from(fw.len() == 2);
            row.damper_extra += usize::from(fw.len() == 2 && model.has_damper_stroke(note));
            row.up_eq += usize::from(same(&fw, &engine_list(sound, false, false, note, vel)));
            row.down_default_eq +=
                usize::from(same(&fw, &engine_list(sound, true, false, note, vel)));
            row.down_optin_eq += usize::from(same(&fw, &engine_list(sound, true, true, note, vel)));
        }
    }
    Some(row)
}

/// CSV of the rows plus per-class totals.
pub fn report(rows: &[SoundRow]) -> String {
    let mut out = String::from(
        "sound,class,points,two_voice,damper_extra,up_eq,down_default_eq,down_optin_eq\n",
    );
    for r in rows {
        out += &format!(
            "{},{},{},{},{},{},{},{}\n",
            r.name,
            r.class,
            r.points,
            r.two_voice,
            r.damper_extra,
            r.up_eq,
            r.down_default_eq,
            r.down_optin_eq
        );
    }
    let mut classes: Vec<u8> = rows.iter().map(|r| r.class).collect();
    classes.sort_unstable();
    classes.dedup();
    out +=
        "# per class: sounds points two_voice damper_extra up_eq down_default_eq down_optin_eq\n";
    for c in classes {
        let sel: Vec<&SoundRow> = rows.iter().filter(|r| r.class == c).collect();
        let sum = |f: fn(&SoundRow) -> usize| sel.iter().map(|r| f(r)).sum::<usize>();
        out += &format!(
            "class {c}: {} {} {} {} {} {} {}\n",
            sel.len(),
            sum(|r| r.points),
            sum(|r| r.two_voice),
            sum(|r| r.damper_extra),
            sum(|r| r.up_eq),
            sum(|r| r.down_default_eq),
            sum(|r| r.down_optin_eq)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_core::piano::npno_level::NpnoLevel;

    /// Minimal image: root 60 mapped from note 60, strokes `(layer, offset, threshold)`.
    /// Offsets mirror `npno_level.rs` (keymap 0xB8, counts 0x648, records 0x75E x 0x76).
    fn image(class: u8, strokes: &[(u8, i16, u8)]) -> Vec<u8> {
        let (keymap, counts, records, len) = (0xB8, 0x648, 0x75E, 0x76);
        let mut d = vec![0u8; records + len * strokes.len()];
        d[keymap..keymap + 128].fill(0xFF);
        d[keymap + 60] = 60;
        d[0x44] = class;
        d[counts + 4..counts + 6].copy_from_slice(&(strokes.len() as u16).to_be_bytes());
        let at = counts + 2 * (3 + 60);
        d[at..at + 2].copy_from_slice(&(strokes.len() as u16).to_be_bytes());
        for (i, &(layer, offset, threshold)) in strokes.iter().enumerate() {
            let r = records + len * i;
            d[r + 4] = layer;
            d[r + 0x34..r + 0x36].copy_from_slice(&offset.to_be_bytes());
            d[r + 0x74] = threshold;
        }
        d
    }

    fn sound(class: u8, strokes: &[(u8, i16, u8)]) -> Arc<NpnoSound> {
        let mut snd = NpnoSound::new_synthetic("t", 60, &[0.2; 64], 48_000.0);
        let stroke = snd.strokes[0].clone();
        snd.strokes = vec![stroke; strokes.len()];
        snd.firmware_level = NpnoLevel::parse(&image(class, strokes)).map(Arc::new);
        Arc::new(snd)
    }

    #[test]
    fn two_voice_class_pedal_down_matches_only_with_the_opt_in() {
        let s = sound(5, &[(0, 66, 0), (0, 56, 16), (0, 36, 46)]);
        let row = census("t", &s).unwrap();
        assert!(row.points > 0 && row.two_voice == row.points);
        assert_eq!(row.up_eq, row.points, "pedal up is the firmware list");
        assert_eq!(row.down_optin_eq, row.points, "opt-in equals the list");
        assert!(
            row.down_default_eq < row.points,
            "default legacy path plays one voice"
        );
    }

    #[test]
    fn damper_stroke_root_counts_the_kind_one_extra_voice() {
        let s = sound(5, &[(0, 66, 0), (0, 56, 16), (1, 50, 0)]);
        let row = census("t", &s).unwrap();
        assert_eq!(row.damper_extra, row.two_voice);
        assert!(row.damper_extra > 0);
    }

    #[test]
    fn single_voice_class_is_equal_on_every_path() {
        let s = sound(4, &[(0, 66, 0), (0, 56, 16)]);
        let row = census("t", &s).unwrap();
        assert_eq!(row.two_voice, 0);
        assert_eq!(row.up_eq, row.points);
        assert_eq!(row.down_optin_eq, row.points);
    }

    #[test]
    fn report_sums_per_class() {
        let a = SoundRow {
            class: 5,
            points: 4,
            down_optin_eq: 4,
            ..SoundRow::default()
        };
        let b = SoundRow {
            class: 5,
            points: 6,
            down_optin_eq: 6,
            ..SoundRow::default()
        };
        let text = report(&[a, b]);
        assert!(text.contains("class 5: 2 10 0 0 0 0 10"), "{text}");
    }
}
