//! The fit view of the baked plate follows the text size option (`BakedZoom::base`, issue #592).

use super::baked_zoom::{fit_size, BakedZoom, ZoomRequest, MAX_ZOOM};
use crate::baked::BakedRect;
use egui::{pos2, vec2, Rect};
use nord_panel::Section;
use std::collections::BTreeMap;

const ASPECT: f32 = 2632.0 / 650.0;

fn view() -> Rect {
    Rect::from_min_size(pos2(0.0, 22.0), vec2(1760.0, 414.0))
}

fn bays() -> BTreeMap<String, BakedRect> {
    let mut m = BTreeMap::new();
    m.insert(
        "bay-organ".into(),
        BakedRect {
            x: 0.095,
            y: 0.048,
            w: 0.187,
            h: 0.572,
            px: [0; 4],
        },
    );
    m
}

#[test]
fn a_larger_base_makes_the_fit_view_that_much_bigger_and_starts_top_left() {
    for base in [1.25_f32, 1.5, 2.0] {
        let mut z = BakedZoom::default();
        z.set_base(base);
        // The view that was at the fit view follows the new base on the next update.
        assert_eq!(z.request, Some(ZoomRequest::Fit));
        z.apply(ZoomRequest::Fit, view(), ASPECT, &bays());
        assert!((z.zoom - base).abs() < 1e-5);
        let plate = z.plate_rect(view(), ASPECT);
        let fitted = fit_size(view().size(), ASPECT);
        assert!((plate.width() - fitted.x * base).abs() < 0.5, "{base}");
        assert!(plate.left() <= view().left() + 0.01, "{base}: {plate:?}");
        assert!(
            plate.left() >= view().left() - 0.01,
            "{base}: starts at the left"
        );
        assert!(plate.right() >= view().right() - 0.01 && plate.bottom() >= view().bottom() - 0.01);
    }
}

#[test]
fn a_zoomed_in_view_keeps_its_zoom_when_the_base_changes() {
    let mut z = BakedZoom::default();
    z.apply(
        ZoomRequest::Section(Section::Organ),
        view(),
        ASPECT,
        &bays(),
    );
    let zoom = z.zoom;
    assert!(zoom > 1.5, "{zoom}");
    z.set_base(1.5);
    assert_eq!(z.request, None);
    assert_eq!(z.zoom, zoom);
    // FIT goes back to the base, not to the whole plate.
    z.apply(ZoomRequest::Fit, view(), ASPECT, &bays());
    assert!((z.zoom - 1.5).abs() < 1e-5);
    assert_eq!(z.base, 1.5);
}

#[test]
fn the_base_is_clamped_to_the_zoom_range_and_a_default_view_is_the_whole_plate() {
    let mut z = BakedZoom::default();
    assert_eq!(z.base, 1.0);
    z.set_base(0.2);
    assert_eq!(z.base, 1.0);
    z.set_base(99.0);
    assert_eq!(z.base, MAX_ZOOM);
    let mut plain = BakedZoom::default();
    plain.apply(ZoomRequest::Fit, view(), ASPECT, &bays());
    assert_eq!((plain.zoom, plain.center), (1.0, vec2(0.5, 0.5)));
}
