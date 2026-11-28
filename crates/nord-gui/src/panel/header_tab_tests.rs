//! Header tabs (Organ, Piano, Synth, Layer Effects): every label and LED printed in a tab stays
//! inside it, measured on the real painted text (no hard-coded widths). Regression for the ORGAN
//! header screenshot: SOLO ran past the tab edge, the ON LED sat above the tab on the panel.

use super::layout_px::to_px;
use super::legibility_tests::audit_of;
use super::view::PANEL_SIZE;
use crate::widgets::probe::Kind;
use egui::{pos2, Rect};
use nord_panel::Section;

fn tab_px(s: Section) -> Rect {
    let t = nord_panel::layout::plate_of(s)
        .and_then(|p| p.tab)
        .expect("section has a tab");
    to_px(t, pos2(0.0, 0.0))
}

const TABBED: [Section; 4] = [
    Section::Organ,
    Section::Piano,
    Section::Synth,
    Section::Effects,
];

#[test]
fn labels_and_leds_printed_in_a_header_tab_stay_inside_it() {
    let a = audit_of(None, PANEL_SIZE);
    for s in TABBED {
        let tab = tab_px(s);
        // Anything whose centre is in the tab's band and not further than 40 px right of it
        // belongs to the header; it must not cross the tab edge.
        let near = |c: egui::Pos2| {
            c.y >= tab.top() - 3.0
                && c.y <= tab.bottom()
                && c.x >= tab.left()
                && c.x <= tab.right() + 40.0
        };
        let labels: Vec<_> = a.labs.iter().filter(|l| near(l.full.center())).collect();
        assert!(labels.len() >= 2, "{s:?}: {} header labels", labels.len());
        for l in &labels {
            assert!(
                tab.contains_rect(l.full),
                "{s:?}: label {:?} {:?} crosses the tab {tab:?}",
                l.text,
                l.full
            );
        }
        // Layer Effects has LAYER FX instead of ON / SOLO.
        assert!(
            s == Section::Effects || labels.iter().any(|l| l.text == "SOLO"),
            "{s:?}: SOLO legend missing"
        );
        let leds: Vec<_> = a
            .recs
            .iter()
            .filter(|r| r.kind == Kind::Led)
            .filter(|r| {
                let c = r.rect.center();
                c.y >= tab.top() - 12.0
                    && c.y <= tab.bottom()
                    && c.x >= tab.left()
                    && c.x <= tab.right() + 10.0
            })
            .collect();
        assert!(!leds.is_empty(), "{s:?}: {} header LEDs", leds.len());
        for r in leds {
            assert!(
                tab.contains_rect(r.rect),
                "{s:?}: LED {:?} outside the tab {tab:?}",
                r.rect
            );
        }
    }
}

#[test]
fn every_header_on_button_carries_its_on_caption() {
    let a = audit_of(None, PANEL_SIZE);
    for s in [Section::Organ, Section::Piano, Section::Synth] {
        let tab = tab_px(s);
        let n = a
            .labs
            .iter()
            .filter(|l| l.text == "ON" && tab.contains_rect(l.full))
            .count();
        assert!(n >= 1, "{s:?}: no ON caption in the header");
    }
}
