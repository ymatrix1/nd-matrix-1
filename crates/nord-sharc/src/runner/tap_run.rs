//! Per-pass data tap of the system run (`sharc-sys --tap ADDR:COUNT --tap-out FILE`): after every
//! pass the listed `DM` windows are appended to a binary file (little-endian `u32`, pass-major,
//! windows in the order given). It reads stage records and frame words that never reach an output
//! lane (the ORG Core2 synth strips end in the channel records `0xC1F96..`, `0xC2942..`), the same
//! way `lane_run` reads the output pointer set. Same message and ping-pong order as `run_system`.
use super::msg::{deliver, Msg};
use super::pass::PassConfig;
use super::system::{lean_pass, set_ping_pong};
use crate::core::Core;

/// Parses `ADDR:COUNT` (decimal or `0x` hex) into a window.
pub fn parse_tap(s: &str) -> Result<(u32, u32), String> {
    let num = |t: &str| {
        t.strip_prefix("0x").map_or_else(
            || t.parse::<u32>().ok(),
            |h| u32::from_str_radix(h, 16).ok(),
        )
    };
    let bad = || format!("bad --tap {s:?} (want ADDR:COUNT)");
    let (a, n) = s.split_once(':').ok_or_else(bad)?;
    match (num(a), num(n)) {
        (Some(a), Some(n)) if n > 0 => Ok((a, n)),
        _ => Err(bad()),
    }
}

/// Runs `total` passes and returns the words of every window after every pass (pass-major) and
/// the number of passes that returned to the poll loop.
pub fn tap_run(
    core: &mut Core,
    cfg: &PassConfig,
    msgs: &[Msg],
    total: u32,
    ping_pong: Option<(u32, u32, u32)>,
    taps: &[(u32, u32)],
) -> Result<(Vec<u32>, u32), String> {
    let mut due: Vec<&Msg> = msgs.iter().collect();
    due.sort_by_key(|m| m.pass);
    let (mut next, mut words, mut passes) = (0, Vec::new(), 0);
    for p in 0..total {
        if let Some(pp) = ping_pong {
            set_ping_pong(core, pp, p);
        }
        while next < due.len() && due[next].pass <= p {
            deliver(core, 0, due[next])?;
            next += 1;
        }
        let (_, idle) = lean_pass(core, cfg);
        if !idle {
            break;
        }
        for &(addr, n) in taps {
            words.extend((0..n).map(|k| core.dm_read(addr.wrapping_add(k)).unwrap_or(0)));
        }
        passes += 1;
    }
    Ok((words, passes))
}

#[cfg(test)]
mod tests {
    use super::super::pass::tests::clearing_core;
    use super::*;

    #[test]
    fn parse_tap_accepts_hex_and_decimal_and_rejects_the_rest() {
        assert_eq!(parse_tap("0xC1F96:8").unwrap(), (0xC1F96, 8));
        assert_eq!(parse_tap("100:0x10").unwrap(), (100, 16));
        for bad in ["", "0x10", "0x10:0", "zz:3", "1:2:3"] {
            assert!(parse_tap(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tap_run_reads_every_window_after_the_pass() {
        let mut c = clearing_core();
        c.dm_write(0xB_0200, 0x3F80_0000).unwrap();
        c.dm_write(0xB_0201, 7).unwrap();
        let cfg = PassConfig {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0002,
            max_steps: 10,
            irq: None,
        };
        let (w, n) = tap_run(&mut c, &cfg, &[], 3, None, &[(0xB_0200, 2), (0xB_0201, 1)]).unwrap();
        // the tiny core idles once, then hits the step limit (see lane_run test)
        assert_eq!(n, 1);
        assert_eq!(w, vec![0x3F80_0000, 7, 7]);
    }
}
