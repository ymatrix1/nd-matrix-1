//! `LayerFxChain::reserve_firmware_stages` (TD-18): parked stage boxes are re-initialised in place
//! on a type change and render bit-identically to boxes built lazily on first use.

use super::*;

fn chain(reserve: bool) -> LayerFxChain {
    let mut fx = LayerFxChain::new(48_000.0);
    fx.firmware_fx = FirmwareFx {
        mod2: true,
        mod1: true,
        ..FirmwareFx::OFF
    };
    fx.mod2_type = Mod2Type::Chorus;
    fx.mod2_rate_raw = Some(80);
    fx.mod2_amount = 0.8;
    fx.mod1_type = Mod1Type::RingMod;
    fx.mod1_rate_raw = Some(60);
    fx.mod1_amount = 0.7;
    if reserve {
        fx.reserve_firmware_stages();
    }
    fx
}

/// Renders 300 frames per (Mod 2 type, Mod 1 type) step through the full chain, switching types
/// between steps like a program change, and returns every output frame.
fn render(fx: &mut LayerFxChain, steps: &[(u8, u8)]) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for &(m2, m1) in steps {
        fx.mod2_type_raw = Some(m2);
        fx.mod1_type_raw = Some(m1);
        for i in 0..300 {
            let x = (i as f32 * 0.05).sin() * 0.4 + 0.1;
            out.push(fx.process_frame([x, 0.7 * x]));
        }
    }
    out
}

#[test]
fn reserved_stages_render_like_lazy_ones() {
    let steps = [
        (4, 4),
        (6, 6),
        (0, 10),
        (9, 5),
        (5, 11),
        (2, 7),
        (7, 8),
        (11, 9),
        (3, 4),
        (4, 6),
        (5, 4),
        (4, 4),
    ];
    let lazy = render(&mut chain(false), &steps);
    let reserved = render(&mut chain(true), &steps);
    assert_eq!(lazy.len(), reserved.len());
    for (i, (a, b)) in lazy.iter().zip(&reserved).enumerate() {
        assert_eq!(
            [a[0].to_bits(), a[1].to_bits()],
            [b[0].to_bits(), b[1].to_bits()],
            "frame {i}"
        );
    }
    assert!(reserved.iter().any(|f| f[0].abs() > 0.01), "not silent");
}

#[test]
fn reserving_twice_keeps_the_boxes_and_a_type_not_covered_falls_back_to_the_engine() {
    let mut fx = chain(true);
    let first = fx.fw_mod2.as_deref().map(|b| b as *const _);
    fx.reserve_firmware_stages();
    assert_eq!(fx.fw_mod2.as_deref().map(|b| b as *const _), first);
    assert!(fx.fw_law.is_some() && fx.fw_vibe.is_some());
    // Type 12 is not a firmware type: the stage is not live and the engine path runs.
    fx.mod2_type_raw = Some(12);
    assert!(fx.process_fw_mod2(0.3, 0.3).is_none());
    assert_eq!(fx.fw_mod2_live, modulation::Mod2Live::None);
}
