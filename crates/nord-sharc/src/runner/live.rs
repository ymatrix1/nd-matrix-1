//! Command interpreter of `sharc-live` (D138): one emulated DSP chip driven pass by pass over a
//! line protocol, so the ARM harness can deliver its messages as the firmware sends them and
//! read back what the chip does (`docs/research/arm-dsp-live-link.md`).
//!
//! | Command | Effect | Reply |
//! |---|---|---|
//! | `M W1 W2` | store word `W2` at data word `W1` before the next pass (a path 2 ARM message) | none (errors surface in the next `P`) |
//! | `P N` | run up to `N` passes, ping-pong and lane read as in `sharc-sys` | `P ...` summary line |
//! | `T 0\|1` | switch per-pass coverage of the probe pcs off or on | none |
//! | `D ADDR COUNT` | read data words | `D ADDR w w ...` (hex) |
//! | `Q` | end | none |
//!
//! Everything is deterministic: no clock, no thread, replies are a pure function of the commands.
pub use super::live_presets::{boot_core, org_preset, pno_preset, preset_named, Preset};
use super::msg::{deliver, Msg};
use super::pass::PassConfig;
use super::periph_run::lane_words;
use super::system::{lean_pass, probe_pass, set_ping_pong};
use super::{parse_number, parse_value};
use crate::core::Core;
use std::fmt::Write as _;

/// One chip and its pass state.
pub struct Live {
    core: Core,
    cfg: PassConfig,
    ping_pong: Option<(u32, u32, u32)>,
    lane: (u32, u32),
    window: Option<(u32, u32, u32)>,
    probes: Vec<u32>,
    probing: bool,
    /// Passes run so far (also the ping-pong phase).
    pub pass: u32,
    halted: bool,
    error: Option<String>,
}

impl Live {
    pub fn new(
        core: Core,
        cfg: PassConfig,
        ping_pong: Option<(u32, u32, u32)>,
        lane: (u32, u32),
        probes: Vec<u32>,
    ) -> Self {
        Self {
            core,
            cfg,
            ping_pong,
            lane,
            window: None,
            probing: !probes.is_empty(),
            probes,
            pass: 0,
            halted: false,
            error: None,
        }
    }

    /// Reads the level statistic from `count` words at `addr + k * stride` (float bits) instead of
    /// an output lane: a synth voice ends in the channel record, not in a lane (`synth-e2e`, D132).
    pub fn with_window(mut self, addr: u32, count: u32, stride: u32) -> Self {
        self.window = Some((addr, count, stride));
        self
    }

    /// The words the level statistic of one pass reads (lane pair or voice window).
    fn level_words(&mut self) -> Vec<u32> {
        if let Some((a, n, st)) = self.window {
            return (0..n)
                .map(|k| self.core.dm_read(a.wrapping_add(k * st)).unwrap_or(0))
                .collect();
        }
        let w = lane_words(&mut self.core, self.lane.0);
        let b = 4 * self.lane.1 as usize;
        w[b..b + 4].to_vec()
    }

    /// Keeps a protocol error for the next `P` reply (no unsolicited output: a client that only
    /// reads after `P` would otherwise deadlock on a full pipe).
    pub fn note_error(&mut self, e: String) {
        self.error.get_or_insert(e);
    }

    /// One protocol line to its reply: `M` and `T` errors wait for the next `P` reply, a bad `P`
    /// or `D` line is answered at once with `ERR ...` so a client never waits for a reply that
    /// will not come.
    pub fn handle(&mut self, line: &str) -> Option<String> {
        match self.exec(line) {
            Ok(reply) => reply,
            Err(e) if matches!(line.trim_start().chars().next(), Some('P' | 'D')) => {
                Some(format!("ERR {e}"))
            }
            Err(e) => {
                self.note_error(e);
                None
            }
        }
    }

    /// Executes one command line; `Ok(None)` for commands without a reply, `Err` for a bad line.
    pub fn exec(&mut self, line: &str) -> Result<Option<String>, String> {
        let f: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| {
            f.get(i)
                .and_then(|s| parse_number(s))
                .ok_or(format!("bad command {line:?}"))
        };
        match f.first().copied() {
            Some("M") => {
                let w2 = f
                    .get(2)
                    .and_then(|s| parse_value(s))
                    .ok_or(format!("bad command {line:?}"))?;
                let m = Msg {
                    pass: self.pass,
                    word1: num(1)?,
                    word2: w2,
                };
                if let Err(e) = deliver(&mut self.core, 0, &m) {
                    self.error.get_or_insert(e);
                }
                Ok(None)
            }
            Some("P") => Ok(Some(self.run_passes(num(1)?))),
            Some("T") => {
                self.probing = num(1)? != 0 && !self.probes.is_empty();
                Ok(None)
            }
            Some("D") => {
                let (addr, n) = (num(1)?, num(2)?);
                let mut out = format!("D {addr:x}");
                for k in 0..n {
                    let v = self.core.dm_read(addr.wrapping_add(k)).unwrap_or(0);
                    let _ = write!(out, " {v:x}");
                }
                Ok(Some(out))
            }
            Some("Q") | None => Ok(None),
            Some(other) => Err(format!("unknown command {other:?}")),
        }
    }

    /// Runs up to `n` passes and summarises them in one line.
    fn run_passes(&mut self, n: u32) -> String {
        let (mut idle, mut steps, mut last) = (0u32, 0u64, 0u64);
        let (mut sumsq, mut cnt, mut peak) = (0f64, 0u64, 0f32);
        let mut hits = vec![0u32; self.probes.len()];
        for _ in 0..n {
            if self.halted {
                break;
            }
            if let Some(pp) = self.ping_pong {
                set_ping_pong(&mut self.core, pp, self.pass);
            }
            let (s, ok) = if self.probing {
                let (s, ok, seen) = probe_pass(&mut self.core, &self.cfg);
                for (h, pc) in hits.iter_mut().zip(&self.probes) {
                    *h += u32::from(seen.contains(pc));
                }
                (s, ok)
            } else {
                lean_pass(&mut self.core, &self.cfg)
            };
            self.pass += 1;
            steps += s;
            last = s;
            if !ok {
                self.halted = true;
                break;
            }
            idle += 1;
            let w = self.level_words();
            for bits in w {
                let x = f32::from_bits(bits);
                if x.is_finite() {
                    sumsq += f64::from(x) * f64::from(x);
                    peak = peak.max(x.abs());
                    cnt += 1;
                }
            }
        }
        let mut out = format!(
            "P req={n} idle={idle} steps={steps} last={last} pass={} halted={} cnt={cnt} sumsq={sumsq:.9e} peak={peak:.9e}",
            self.pass,
            u8::from(self.halted)
        );
        if self.probing {
            let list: Vec<String> = self
                .probes
                .iter()
                .zip(&hits)
                .map(|(p, h)| format!("{p:x}:{h}"))
                .collect();
            let _ = write!(out, " hits={}", list.join(","));
        }
        if let Some(e) = self.error.take() {
            let _ = write!(out, " error={}", e.replace(' ', "_"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::pass::tests::clearing_core;
    use super::*;

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
    fn message_then_dump_round_trips_a_data_word() {
        let mut l = live(vec![]);
        assert_eq!(l.exec("M 0xB0200 0x1234").unwrap(), None);
        assert_eq!(l.exec("D 0xB0200 2").unwrap().unwrap(), "D b0200 1234 0");
        assert_eq!(l.exec("M 0xB0201 f1.0").unwrap(), None);
        assert_eq!(l.exec("D 0xB0201 1").unwrap().unwrap(), "D b0201 3f800000");
    }

    #[test]
    fn passes_report_idle_halt_lane_level_and_probe_hits() {
        let mut l = live(vec![0x12_0000, 0x12_7777]);
        // pointer set 2 -> 0xB0200, pair 1 words at +4..+7
        // OUT_POINTERS = 0xB0159: set 2 is word 0xB015B
        l.exec("M 0xB015B 0xB0200").unwrap();
        for (k, v) in [(4, "f0.5"), (5, "f-0.25"), (6, "f0.125"), (7, "f1.0")] {
            l.exec(&format!("M {} {v}", 0xB_0200 + k)).unwrap();
        }
        let r = l.exec("P 3").unwrap().unwrap();
        // the tiny core idles once, then hits the step limit (same as lane_run's test)
        assert!(r.starts_with("P req=3 idle=1 "), "{r}");
        assert!(r.contains("halted=1") && r.contains("cnt=4"), "{r}");
        assert!(r.contains("peak=1.000000000e0"), "{r}");
        assert!(r.contains("hits=120000:1,127777:0"), "{r}");
        // halted: no further pass runs
        let r2 = l.exec("P 5").unwrap().unwrap();
        assert!(r2.contains("idle=0") && r2.contains("pass=2"), "{r2}");
    }

    #[test]
    fn a_voice_window_replaces_the_lane_in_the_level() {
        let mut l = live(vec![]).with_window(0xB_0300, 2, 2);
        for (k, v) in [(0, "f0.5"), (1, "f9.0"), (2, "f-0.75"), (3, "f9.0")] {
            l.exec(&format!("M {} {v}", 0xB_0300 + k)).unwrap();
        }
        let r = l.exec("P 1").unwrap().unwrap();
        // words 0 and 2 only (stride 2): peak 0.75, the 9.0 words are skipped
        assert!(
            r.contains("cnt=2") && r.contains("peak=7.500000000e-1"),
            "{r}"
        );
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
    fn bad_p_and_d_lines_are_answered_and_bad_m_lines_are_deferred() {
        let mut l = live(vec![]);
        assert!(l.handle("P x").unwrap().starts_with("ERR "));
        assert!(l.handle("D").unwrap().starts_with("ERR "));
        assert_eq!(l.handle("M zz 1"), None);
        assert!(l.handle("P 1").unwrap().contains("error=bad_command"));
    }

    #[test]
    fn server_passes_equal_lane_run_on_the_same_messages() {
        use super::super::periph_run::{lane_run, OUT_POINTERS};
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
}
