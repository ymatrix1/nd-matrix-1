//! Engine-level tests of the opt-in oracle envelope law (`fw_adr_law.rs`, D210).

use super::fw_velocity::key_byte;
use super::*;

fn layer(law: bool) -> SynthLayer {
    let mut s = SynthLayer::new(48_000.0);
    s.chain = SynthChain::FirmwareStrip;
    s.attack_sec = 0.05;
    s.decay_sec = 0.3;
    s.sustain_level = 0.0;
    s.release_sec = 0.2;
    s.fw_adr_law = law;
    s
}

/// Passes until the amp chain of a voice keyed at MIDI velocity `vel` reaches `level`.
fn passes_to(s: &mut SynthLayer, vel: u8, level: f32) -> usize {
    let p = s.strip_params();
    let mut v = s.new_strip_voice(key_byte(f32::from(vel) / 127.0));
    (1..200_000)
        .find(|_| v.advance(&p, true).0 >= level)
        .expect("attack finishes")
}

#[test]
fn off_by_default_and_the_voice_uses_the_shared_record() {
    assert!(!SynthLayer::new(48_000.0).fw_adr_law);
    let s = layer(false);
    assert!(s.new_strip_voice(10).amp_attack_b.is_none());
    assert_ne!(s.clone().strip_params().amp.decay.thr, 0.5);
}

#[test]
fn soft_keys_attack_slower_and_full_velocity_matches_the_shared_record() {
    let mut s = layer(true);
    let soft = passes_to(&mut s, 20, 0.5);
    let hard = passes_to(&mut s, 127, 0.5);
    let off = passes_to(&mut layer(false), 20, 0.5);
    assert!(soft > hard, "soft {soft} hard {hard}");
    assert_eq!(hard, off, "g = 127 is unscaled");
    assert!(soft as f32 > 1.1 * hard as f32);
}

#[test]
fn decay_stops_at_half_in_both_chains_and_in_hold() {
    let mut s = layer(true);
    let p = s.strip_params();
    assert_eq!(p.amp.decay.thr, 0.5);
    assert_eq!(p.filter.decay.thr, 0.5);
    let mut v = s.new_strip_voice(120);
    let mut last = (0.0, 0.0);
    for _ in 0..60_000 {
        last = v.advance(&p, true);
    }
    assert!(
        (last.0 - 0.5).abs() < 1e-3 && (last.1 - 0.5).abs() < 1e-3,
        "{last:?}"
    );
    s.decay_sec = 45.0;
    assert_eq!(
        s.strip_params().amp.decay.thr,
        0.5,
        "Hold keeps the oracle record too"
    );
}

#[test]
fn toggling_the_flag_invalidates_the_parameter_cache() {
    let mut s = layer(false);
    let off = s.strip_params();
    s.fw_adr_law = true;
    assert_ne!(s.strip_params(), off);
}

#[test]
fn a_played_note_gets_the_scaled_attack() {
    let mut s = layer(true);
    s.note_on(60, 20);
    let b = s
        .voices
        .iter()
        .find(|v| v.active)
        .and_then(|v| v.strip.amp_attack_b);
    let shared = s.strip_params().amp.attack.b;
    assert!(b.is_some_and(|b| b < shared), "{b:?} vs {shared}");
}

#[test]
fn settled_tables_feed_the_records_of_stored_knobs() {
    let k = EnvKnobs {
        attack: 24,
        decay: 24,
        release: 4,
    };
    let legacy = StripEnvRecords::from_knobs_with(k, 0.0, LEGACY_TABLES);
    let settled = StripEnvRecords::from_knobs_with(k, 0.0, SETTLED_TABLES);
    assert_eq!(
        legacy,
        StripEnvRecords::from_knobs(k, 0.0),
        "default = legacy"
    );
    // knob 24 is index -40: the oracle installer words after the 16-pass refresh.
    assert_eq!(settled.attack.b, f32::from_bits(0x3b0e_6533));
    assert!((settled.decay.a - 0.996_174_7).abs() < 1e-7);
    assert!(
        settled.release.a < settled.decay.a,
        "index -60 falls faster than -40"
    );
    assert!(1.0 - settled.decay.a < 0.9 * (1.0 - legacy.decay.a));
    assert!(settled.attack.b < 0.5 * legacy.attack.b + 1e-3 || settled.attack.b != legacy.attack.b);
}

#[test]
fn default_tables_are_still_the_legacy_sweep() {
    assert_eq!((DEFAULT_TABLES.attack)(-60.0), attack_coeff(-60.0));
    assert_eq!(
        (DEFAULT_TABLES.decay_release)(-60.0),
        decay_release_coeff(-60.0)
    );
    assert_ne!((SETTLED_TABLES.attack)(-60.0), attack_coeff(-60.0));
}

#[test]
fn the_oracle_law_reads_the_settled_decay_and_release() {
    // Stored knobs; the panel path maps times through the stale table, which lands on the plateau
    // anchors where both tables agree (D212 follow-up: invert the settled table).
    let mut moved = 0;
    for knob in [10u8, 22, 30, 40, 52, 70, 90] {
        let e = EnvKnobs {
            attack: knob,
            decay: knob,
            release: knob,
        };
        let (mut on, mut off) = (layer(true), layer(false));
        for s in [&mut on, &mut off] {
            s.knob_laws = true;
            s.strip_knobs = Some(StripKnobs { amp: e, filter: e });
        }
        let (a, b) = (on.strip_params(), off.strip_params());
        // the rate `1 - a` is never faster than the stale sweep (it read too high between anchors)
        assert!(
            1.0 - a.amp.decay.a <= 1.0 - b.amp.decay.a + 1e-9,
            "knob {knob}"
        );
        moved += usize::from(a.amp.decay.a != b.amp.decay.a);
        moved += usize::from(a.filter.release.a != b.filter.release.a);
    }
    assert!(moved >= 8, "{moved} of 14 decay / release rates moved");
}
