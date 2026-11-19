//! Per-program Piano Touch census (`piano_touch_census` binary): which factory programs carry
//! Heavy or Light, as decoded by the project's own program decoder. Derived numbers and program
//! names only, no firmware data.

use nord_core::cbin::ProgramDecoded;
use nord_core::piano::npno_level::NpnoLevel;
use nord_core::piano::touch_curve::TouchCurve;
use nord_core::piano::VelocityTouch;
use std::fmt::Write;

/// Touch facts of one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramTouch {
    pub name: String,
    /// Bank letter `A..` and 1-based location in the bank (`A:1`).
    pub slot: String,
    pub touch_a: VelocityTouch,
    pub touch_b: VelocityTouch,
    pub layer_a_on: bool,
    pub layer_b_on: bool,
    /// The piano section is enabled in scene I or II (`section_enable & 0b010`).
    pub piano_section_on: bool,
    /// Archive path of the layer A `.npno` sound (`None`: not resolved).
    pub sound_a: Option<String>,
}

/// `A:1` style slot label of `(bank_index, location_index)`.
pub fn slot_label(bank: u16, loc: u16) -> String {
    let letter = char::from(b'A' + u8::try_from(bank).unwrap_or(25).min(25));
    format!("{letter}:{}", u32::from(loc) + 1)
}

/// Facts of a decoded program.
pub fn facts(prog: &ProgramDecoded) -> ProgramTouch {
    let g = &prog.global;
    ProgramTouch {
        name: prog.name.clone(),
        slot: slot_label(prog.bank_index, prog.location_index),
        touch_a: prog.piano.layer_a.touch,
        touch_b: prog.piano.layer_b.touch,
        layer_a_on: prog.piano.layer_a.enabled,
        layer_b_on: prog.piano.layer_b.enabled,
        piano_section_on: (g.section_enable_i | g.section_enable_ii) & 0b010 != 0,
        sound_a: None,
    }
}

impl ProgramTouch {
    /// The Touch of every piano layer that is enabled in a section that is on.
    pub fn active_touches(&self) -> Vec<VelocityTouch> {
        let mut v = Vec::new();
        if self.piano_section_on && self.layer_a_on {
            v.push(self.touch_a);
        }
        if self.piano_section_on && self.layer_b_on {
            v.push(self.touch_b);
        }
        v
    }
}

/// Header of the per-program CSV (`piano_touch_census --csv`), read by
/// `tools/arm_program_touch_bits.py`.
pub const CSV_HEADER: &str = "entry,slot,name,touch_a,touch_b,layer_a_on,layer_b_on,section_on";

/// CSV row of a program: archive entry, slot, name, the decoded layer Touch as the edit value
/// (0 heavy, 1 medium, 2 light, the value the firmware recall holds) and the in-use flags.
pub fn csv_row(entry: &str, r: &ProgramTouch) -> String {
    format!(
        "{entry},{},{},{},{},{},{},{}",
        r.slot,
        r.name.replace(',', " "),
        index(r.touch_a),
        index(r.touch_b),
        u8::from(r.layer_a_on),
        u8::from(r.layer_b_on),
        u8::from(r.piano_section_on)
    )
}

fn index(t: VelocityTouch) -> usize {
    match t {
        VelocityTouch::Heavy => 0,
        VelocityTouch::Medium => 1,
        VelocityTouch::Light => 2,
    }
}

/// Text report: counts of the stored layer-A touch (all programs, the number the firmware recall
/// check compares with), counts over programs that use a piano layer, and the Heavy / Light lists.
pub fn report(rows: &[ProgramTouch]) -> String {
    let mut a = [0usize; 3];
    let mut b = [0usize; 3];
    let mut used = [0usize; 3];
    let (mut heavy, mut light) = (String::new(), String::new());
    for r in rows {
        a[index(r.touch_a)] += 1;
        b[index(r.touch_b)] += 1;
        let act = r.active_touches();
        for t in &act {
            used[index(*t)] += 1;
        }
        for (t, list) in [
            (VelocityTouch::Heavy, &mut heavy),
            (VelocityTouch::Light, &mut light),
        ] {
            if act.contains(&t) {
                let _ = write!(
                    list,
                    "{} {} [{}]; ",
                    r.slot,
                    r.name,
                    r.sound_a.as_deref().unwrap_or("-")
                );
            }
        }
    }
    let mut out = String::new();
    let _ = writeln!(out, "programs: {}", rows.len());
    let _ = writeln!(
        out,
        "layer A touch (Heavy/Medium/Light): {}/{}/{}",
        a[0], a[1], a[2]
    );
    let _ = writeln!(
        out,
        "layer B touch (Heavy/Medium/Light): {}/{}/{}",
        b[0], b[1], b[2]
    );
    let _ = writeln!(
        out,
        "piano layers in use (section on, layer on): {}/{}/{}",
        used[0], used[1], used[2]
    );
    let _ = writeln!(out, "Heavy in use: {heavy}");
    let _ = writeln!(out, "Light in use: {light}");
    out
}

/// One `touch_model` row text: `velocity T X amplitude_q23...` (one amplitude per voice).
pub fn format_row(velocity: u8, position: u8, x: i16, amplitudes: &[u32]) -> String {
    let mut row = format!("{velocity} {position} {x}");
    for a in amplitudes {
        let _ = write!(row, " {a}");
    }
    row
}

/// The `touch_model` row of a note under a Touch curve.
pub fn model_row(level: &NpnoLevel, note: u8, velocity: u8, touch: TouchCurve) -> String {
    let amps: Vec<u32> = level
        .voices_touch(note, velocity, touch)
        .iter()
        .map(|v| v.amplitude_q23)
        .collect();
    format_row(
        velocity,
        touch.velocity_position(velocity),
        touch.x(velocity),
        &amps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, a: VelocityTouch, on: bool) -> ProgramTouch {
        ProgramTouch {
            name: name.into(),
            slot: slot_label(0, 0),
            touch_a: a,
            touch_b: VelocityTouch::Medium,
            layer_a_on: on,
            layer_b_on: false,
            piano_section_on: true,
            sound_a: None,
        }
    }

    #[test]
    fn format_row_lists_position_attenuation_and_every_voice() {
        assert_eq!(format_row(64, 88, 74, &[10, 20]), "64 88 74 10 20");
        assert_eq!(format_row(1, 200, 170, &[]), "1 200 170");
        // the Heavy curve at velocity 64 (research table: T 88, X 74)
        let t = TouchCurve::Heavy;
        assert_eq!(
            (t.velocity_position(64), t.x(64)),
            (88, 74),
            "the row header comes from the curve"
        );
    }

    #[test]
    fn csv_row_writes_edit_values_and_flags() {
        let mut r = row("Pearl, Upright", VelocityTouch::Heavy, true);
        r.touch_b = VelocityTouch::Light;
        assert_eq!(
            csv_row("Program/Bank A/Pearl Upright.ns4p", &r),
            "Program/Bank A/Pearl Upright.ns4p,A:1,Pearl  Upright,0,2,1,0,1"
        );
        assert_eq!(CSV_HEADER.split(',').count(), 8);
    }

    #[test]
    fn slot_label_is_bank_letter_and_one_based_location() {
        assert_eq!(slot_label(0, 0), "A:1");
        assert_eq!(slot_label(5, 63), "F:64");
    }

    #[test]
    fn report_counts_and_lists_only_programs_that_use_a_piano_layer() {
        let rows = [
            row("Heavy On", VelocityTouch::Heavy, true),
            row("Heavy Off", VelocityTouch::Heavy, false),
            row("Light On", VelocityTouch::Light, true),
        ];
        let text = report(&rows);
        assert!(
            text.contains("layer A touch (Heavy/Medium/Light): 2/0/1"),
            "{text}"
        );
        assert!(
            text.contains("in use (section on, layer on): 1/0/1"),
            "{text}"
        );
        assert!(text.contains("Heavy in use: A:1 Heavy On [-];"), "{text}");
        assert!(!text.contains("Heavy Off"), "{text}");
    }

    #[test]
    fn facts_reads_the_decoded_program() {
        let mut p = ProgramDecoded::default();
        p.piano.layer_a.touch = VelocityTouch::Light;
        p.bank_index = 2;
        p.location_index = 9;
        let f = facts(&p);
        assert_eq!((f.touch_a, f.slot.as_str()), (VelocityTouch::Light, "C:10"));
        assert!(f.piano_section_on && f.layer_a_on);
        assert_eq!(f.active_touches(), vec![VelocityTouch::Light]);
    }
}
