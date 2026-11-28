//! The `library_program` host parameter: which program of the user's `.ns4b` sound library an
//! instance plays, as a value the host automates and saves with the project (so a reloaded
//! project sounds the same, and a DAW without a plugin window can still pick a sound).
//!
//! Value 0 is Off (the panel / host parameters alone, as before). Values 1..=512 are the hardware
//! locations A:11 .. H:88 (8 banks of 8 pages of 8 slots, the flat location + 1), shown with the
//! display's own labels.

use nih_plug::prelude::*;
use std::sync::Arc;

/// Banks, pages per bank and slots per page of the Program area (VERIFIED: the display labels
/// `A:11`..`H:88`, `nord_gui::panel::bay_program::store`).
const BANKS: i32 = 8;
const PER_BANK: i32 = 64;
/// Program locations the parameter can address.
pub const TOTAL: i32 = BANKS * PER_BANK;

/// Host parameter id (the `library` family is never held by a program load).
pub const ID: &str = "library_program";

/// Flat location (0-based) of parameter value `v`, `None` for Off or out of range.
pub fn location(v: i32) -> Option<u16> {
    (1..=TOTAL).contains(&v).then(|| (v - 1) as u16)
}

/// Display label of value `v`: `Off`, else `A:11`..`H:88`.
pub fn label(v: i32) -> String {
    match location(v) {
        None => "Off".to_string(),
        Some(i) => {
            let i = i32::from(i);
            format!(
                "{}:{}{}",
                (b'A' + (i / PER_BANK) as u8) as char,
                (i % PER_BANK) / 8 + 1,
                i % 8 + 1
            )
        }
    }
}

/// Parses what [`label`] prints (also a bare number, as hosts type it).
pub fn parse(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("off") {
        return Some(0);
    }
    if let Ok(n) = s.parse::<i32>() {
        return (0..=TOTAL).contains(&n).then_some(n);
    }
    let b = s.as_bytes();
    if b.len() != 4 || b[1] != b':' {
        return None;
    }
    let bank = b[0].to_ascii_uppercase().checked_sub(b'A')?;
    let page = b[2].checked_sub(b'1')?;
    let slot = b[3].checked_sub(b'1')?;
    if i32::from(bank) >= BANKS || page >= 8 || slot >= 8 {
        return None;
    }
    Some(i32::from(bank) * PER_BANK + i32::from(page) * 8 + i32::from(slot) + 1)
}

/// The parameter: Off .. H:88, default A:11 (the first program of the factory bank, so an
/// instance with a saved library sounds like the keyboard at power-on).
pub fn new_param() -> IntParam {
    IntParam::new("Program", 1, IntRange::Linear { min: 0, max: TOTAL })
        .with_value_to_string(Arc::new(label))
        .with_string_to_value(Arc::new(parse))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_display_and_round_trip() {
        assert_eq!(label(0), "Off");
        assert_eq!(label(1), "A:11");
        assert_eq!(label(8), "A:18");
        assert_eq!(label(9), "A:21");
        assert_eq!(label(64), "A:88");
        assert_eq!(label(65), "B:11");
        assert_eq!(label(TOTAL), "H:88");
        assert_eq!(label(TOTAL + 1), "Off");
        for v in 0..=TOTAL {
            assert_eq!(parse(&label(v)), Some(v), "{v}");
        }
        assert_eq!(parse(" b:11 "), Some(65));
        assert_eq!(parse("17"), Some(17));
        assert_eq!(parse("I:11"), None);
        assert_eq!(parse("A:19"), None);
        assert_eq!(parse("A:1"), None);
        assert_eq!(parse("600"), None);
        assert_eq!(location(0), None);
        assert_eq!(location(1), Some(0));
        assert_eq!(location(TOTAL), Some(511));
    }

    #[test]
    fn the_parameter_defaults_to_the_first_factory_program() {
        let p = new_param();
        assert_eq!(p.value(), 1);
        assert_eq!(p.step_count(), Some(TOTAL as usize));
        assert_eq!(p.normalized_value_to_string(0.0, false), "Off");
        assert_eq!(p.string_to_normalized_value("H:88"), Some(1.0));
    }
}
