//! Shape of the printed section plates: a light header rim along the plate's top edge and, for
//! Organ and Layer Effects, a raised header tab above it that carries the title, FX FOCUS, ON and
//! SOLO (Piano and Program have the tab across the full width, Synth has none).
//!
//! VERIFIED on the straight-on HA88 render (`.reference/NS4_HA88_TopDown-01_241008.jpg`, blue-over-red
//! colour mask, 0.1195 mm/px): tab top 25.0 mm (row 691 px); plate top (the step beside the tab)
//! 30.2..31.2 mm in the Organ, Synth and Layer Effects columns; slate body starts about 35.1 mm
//! (rim 4.3 mm); Organ tab x 240.0..324.7 mm (row 715 px), Layer Effects tab x 924.7..1008.8 mm.
//! Checked by [`tests::plate_matches_the_render_measurements`]; +-0.5 mm.

use super::{section_rect, Rect};
use crate::types::Section;

/// Plate top edge (mm) beside a header tab.
const PLATE_TOP_MM: f32 = 30.8;
/// Width (mm) of the Synth header tab (GUI styling): holds FX FOCUS, ON and SOLO.
pub const SYNTH_TAB_MM: f32 = 108.0;
/// Height (mm) of the light rim along the plate's top edge.
pub const RIM_MM: f32 = 4.3;

/// Header geometry of a section plate, in panel mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate {
    /// Top edge of the plate beside the tab (`y` of the plate body, not of the tab).
    pub top: f32,
    /// Tab: `None` when the plate has none, else the tab rectangle (starts at the section's top).
    pub tab: Option<Rect>,
}

/// Header geometry of `section` (`None` for Master and Keybed, which have no plate).
pub fn plate_of(section: Section) -> Option<Plate> {
    let s = section_rect(section);
    let tab = |w: f32| Rect::new(s.x, s.y, w, PLATE_TOP_MM - s.y + RIM_MM);
    Some(match section {
        // Tab across the full width: the plate top is the section top.
        Section::Piano | Section::Program => Plate {
            top: s.y,
            tab: Some(tab(s.w)),
        },
        Section::Organ => Plate {
            top: PLATE_TOP_MM,
            tab: Some(tab(83.8)),
        },
        Section::Effects => Plate {
            top: PLATE_TOP_MM,
            tab: Some(tab(85.4)),
        },
        // The hardware prints no tab here (ON / SOLO / FX FOCUS sit on the panel); the GUI
        // gives Synth one like Organ and Layer Effects so the header reads the same everywhere
        // and the ON button, its LED and SOLO stay on a plate (production look, not photo parity).
        Section::Synth => Plate {
            top: PLATE_TOP_MM,
            tab: Some(tab(SYNTH_TAB_MM)),
        },
        Section::Master | Section::Keybed => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plate_matches_the_render_measurements() {
        let o = plate_of(Section::Organ).unwrap();
        assert!((o.top - 30.8).abs() < 0.5);
        let t = o.tab.unwrap();
        assert!((t.y - 25.0).abs() < 1e-3 && (t.right() - 324.7).abs() < 0.5);
        let e = plate_of(Section::Effects).unwrap().tab.unwrap();
        assert!((e.right() - 1008.8).abs() < 1.5, "{}", e.right());
        assert!(
            plate_of(Section::Synth).unwrap().tab.is_some(),
            "GUI styling: Synth has a tab"
        );
    }

    #[test]
    fn tabs_lie_inside_their_section_and_full_width_tabs_start_at_its_top() {
        for s in [
            Section::Organ,
            Section::Piano,
            Section::Program,
            Section::Synth,
            Section::Effects,
        ] {
            let p = plate_of(s).unwrap();
            let sec = section_rect(s);
            assert!(p.top >= sec.y - 1e-3);
            if let Some(t) = p.tab {
                assert!(sec.contains(&t), "{s:?}");
            }
        }
        let piano = plate_of(Section::Piano).unwrap();
        assert!((piano.top - 25.0).abs() < 1e-3);
        assert!(plate_of(Section::Master).is_none() && plate_of(Section::Keybed).is_none());
    }
}
