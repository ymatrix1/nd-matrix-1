//! Protocol tests of `live.rs` that need no firmware: error lines and the equality of the server
//! passes with `lane_run` (split out to keep `live.rs` under the module size rule).
use super::live::Live;
use super::msg::Msg;
use super::pass::tests::clearing_core;
use super::pass::PassConfig;

fn live(probes: Vec<u32>) -> Live {
    let cfg = PassConfig {
        trigger: 0xB_00E2,
        idle_pc: 0x12_0002,
        max_steps: 10,
        irq: None,
    };
    Live::new(clearing_core(), cfg, None, (2, 1), probes)
}

#[test]
fn bad_lines_are_errors_and_quit_is_silent() {
    let mut l = live(vec![]);
    assert!(l.exec("X 1").is_err());
    l.note_error("bad command".into());
    assert!(l
        .exec("P 1")
        .unwrap()
        .unwrap()
        .contains("error=bad_command"));
    assert!(l.exec("M zz 1").is_err());
    assert!(l.exec("P").is_err());
    assert_eq!(l.exec("Q").unwrap(), None);
    assert_eq!(
        l.exec("T 1").unwrap(),
        None,
        "no probes: coverage stays off"
    );
}

#[test]
fn server_passes_equal_lane_run_on_the_same_messages() {
    use super::periph_run::{lane_run, OUT_POINTERS};
    let cfg = PassConfig {
        trigger: 0xB_00E2,
        idle_pc: 0x12_0002,
        max_steps: 10,
        irq: None,
    };
    let msgs = [
        (OUT_POINTERS + 2, 0xB_0200),
        (0xB_0204, 0.5f32.to_bits()),
        (0xB_0205, (-0.25f32).to_bits()),
        (0xB_0206, 0.125f32.to_bits()),
        (0xB_0207, 1.0f32.to_bits()),
    ];
    let list: Vec<Msg> = msgs
        .iter()
        .map(|&(w1, w2)| Msg {
            pass: 0,
            word1: w1,
            word2: w2,
        })
        .collect();
    let want = lane_run(&mut clearing_core(), &cfg, &list, 3, None, (2, 1)).unwrap();
    let mut l = Live::new(clearing_core(), cfg, None, (2, 1), vec![]);
    for (w1, w2) in msgs {
        l.exec(&format!("M {w1:#x} {w2:#x}")).unwrap();
    }
    let r = l.exec("P 3").unwrap().unwrap();
    let (peak, rms) = want.level();
    assert!(r.contains(&format!("idle={}", want.passes)), "{r}");
    assert!(
        r.contains(&format!("cnt={}", want.left.len() + want.right.len())),
        "{r}"
    );
    assert!(r.contains(&format!("peak={:.9e}", peak)), "{r}");
    let sumsq = rms * rms * (want.left.len() + want.right.len()) as f64;
    assert!(
        r.contains(&format!("sumsq={:.9e}", sumsq)),
        "{r} vs {sumsq:e}"
    );
}
