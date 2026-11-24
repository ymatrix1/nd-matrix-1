//! CPU cost of the firmware organ stages (B3 Bass, Farfisa): seconds of CPU per second of audio.
//!
//! `cargo run --release -p nord-tools --bin organ_fw_bench` (research s17, TD-15). One chord of
//! three keys is held for 4 s of 48 kHz audio; the figure is `wall time / audio time`.

use std::time::Instant;

use nord_core::organ::{OrganLayer, OrganModel, ToneWheelSource};

fn layer(bass: bool) -> OrganLayer {
    let mut o = OrganLayer::new(48_000.0);
    o.rotary_enabled = false;
    if bass {
        o.model = OrganModel::B3;
        o.b3_bass_mode = true;
        o.tonewheel_source = ToneWheelSource::B3Wheels;
        o.set_b3_bass_processor(true);
    } else {
        o.model = OrganModel::Farfisa;
        o.tonewheel_source = ToneWheelSource::FarfisaWheels;
        o.set_farfisa_processor(true);
    }
    let loaded = if bass {
        o.b3_bass_firmware_loaded()
    } else {
        o.farfisa_firmware_loaded()
    };
    if !loaded {
        // D128: the stages are built from the user's OS update file (`NS4_WAVE_TABLES`)
        eprintln!("no OS update file: the firmware stage is off, the figure below is not its cost");
    }
    for k in [60, 64, 67] {
        o.note_on(k);
    }
    o
}

/// Renders `frames` frames of one stage; returns (peak, FNV hash of every output sample).
fn render(bass: bool, frames: usize) -> (f32, u64) {
    let mut o = layer(bass);
    let mut peak = 0.0f32;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..frames {
        let f = o.render_frame();
        peak = peak.max(f[0].abs());
        for v in f {
            hash = (hash ^ u64::from(v.to_bits())).wrapping_mul(0x100_0000_01b3);
        }
    }
    (peak, hash)
}

fn main() {
    let frames = 4 * 48_000;
    for (name, bass) in [("b3 bass", true), ("farfisa", false)] {
        let t = Instant::now();
        let (peak, hash) = render(bass, frames);
        let cpu = t.elapsed().as_secs_f64() / 4.0;
        println!("{name}: {cpu:.3} s CPU per second of audio (peak {peak:.4}, fnv {hash:#018x})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_stages_sound_and_render_deterministically() {
        for bass in [true, false] {
            let (peak, hash) = render(bass, 2_000);
            assert!(peak > 0.0 && peak.is_finite());
            assert_eq!(render(bass, 2_000), (peak, hash));
        }
    }
}
