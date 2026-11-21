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
//! | `L PATH` | load a `.npno` as the SMP flash image behind the external banks, chain model on (`live_flash`) | `L words=N` |
//! | `F REC11` | place the file for the descriptor directory index `REC11` and write the linear page directory | `F base=.. dir=..` |
//! | `C PATH` | capture the tapped words of every later pass to PATH (f32 little endian) | `C ok` |
//! | `H 0\|1` | SMP ring tap: 1 reads the half the pointer does not hold on entry | `H n` |
//! | `E [FIRST COUNT]` | external-port DMA chain totals (`--epchain`); with a range also the executed TCBs | `E chains=.. tcbs=..` (+ ` ext:int:count:dir` items, hex) |
//! | `S` | register snapshots taken at the first `--probe` pc inside the audio interrupts since the last `S` (max 256) | `S r=..;i=..;l=.. ...` |
//! | `G` | core registers `r`, `s`, `i`, `l` (hex) | `G pc=.. r=.. s=.. i=.. l=..` |
//! | `W ADDR N` | watch data writes to `ADDR..ADDR+N` (at most 4096 hits kept); `W` alone reads and clears the hits | `W pc:addr:value ...` (hex, `pc` = writing instruction) |
//! | `Q` | end | none |
//!
//! Everything is deterministic: no clock, no thread, replies are a pure function of the commands.
pub use super::live_presets::{boot_core, org_preset, pno_preset, preset_named, sy_preset, Preset};
use super::live_sy::SyModel;
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
    /// The SY chip: messages go to its FIFO queue and a pass is audio interrupts (D153).
    sy: Option<SyModel>,
    /// `C PATH`: every tapped word of each pass is appended to this file as f32 little endian (local audio, never committed).
    capture: Option<std::fs::File>,
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
            sy: None,
            capture: None,
        }
    }

    /// Serves the SY1SY2 chip: `M` queues FIFO pairs, `P` runs `model.isrs_per_pass` frames.
    pub fn with_sy(mut self, mut model: SyModel) -> Self {
        model.probes = self.probes.clone();
        model.probe_hits = vec![0; self.probes.len()];
        self.sy = Some(model);
        self
    }

    /// Reads the level statistic from `count` words at `addr + k * stride` (float bits) instead of
    /// an output lane: a synth voice ends in the channel record, not in a lane (`synth-e2e`, D132).
    pub fn with_window(mut self, addr: u32, count: u32, stride: u32) -> Self {
        self.window = Some((addr, count, stride));
        self
    }

    /// The words the level statistic of one pass reads (lane pair or voice window).
    fn level_words(&mut self) -> Vec<u32> {
        if let Some(sy) = &self.sy {
            return sy.level_words();
        }
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
            Err(e)
                if matches!(
                    line.trim_start().chars().next(),
                    Some('P' | 'D' | 'L' | 'F' | 'C' | 'H' | 'S' | 'W' | 'G')
                ) =>
            {
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
                if let Some(sy) = &mut self.sy {
                    sy.send(num(1)?, w2);
                    return Ok(None);
                }
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
            Some("L") => {
                let path = line.trim_start().strip_prefix('L').unwrap_or("").trim();
                super::live_flash::load(&mut self.core, path).map(Some)
            }
            Some("C") => {
                let path = line.trim_start().strip_prefix('C').unwrap_or("").trim();
                let f = std::fs::File::create(path).map_err(|e| format!("C {path}: {e}"))?;
                self.capture = Some(f);
                Ok(Some("C ok".to_string()))
            }
            Some("H") => {
                let on = num(1)? != 0;
                match &mut self.sy {
                    Some(sy) => sy.other_half = on,
                    None => return Err("H: not an SMP chip".to_string()),
                }
                Ok(Some(format!("H {}", u8::from(on))))
            }
            Some("S") => match &mut self.sy {
                Some(sy) => Ok(Some(format!(
                    "S {}",
                    std::mem::take(&mut sy.snaps).join(" ")
                ))),
                None => Err("S: not a sample chip".to_string()),
            },
            Some("G") => {
                let c = &self.core;
                let f = |n: &str, a: &[u32; 16]| {
                    format!(
                        " {n}={}",
                        a.iter()
                            .map(|v| format!("{v:x}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                Ok(Some(format!(
                    "G pc={:x}{}{}{}{}",
                    c.pc,
                    f("r", &c.r),
                    f("s", &c.s),
                    f("i", &c.i),
                    f("l", &c.l)
                )))
            }
            Some("W") => Ok(Some(self.watch_cmd(num(1).ok(), num(2).ok()))),
            Some("F") => super::live_flash::place(&mut self.core, num(1)?).map(Some),
            Some("E") => Ok(Some(self.epchain_report(num(1).ok(), num(2).ok()))),
            Some("Q") | None => Ok(None),
            Some(other) => Err(format!("unknown command {other:?}")),
        }
    }

    /// `W` reply: arms the write watch (`ADDR N`) or reports and clears the hits.
    fn watch_cmd(&mut self, addr: Option<u32>, len: Option<u32>) -> String {
        if let (Some(a), Some(n)) = (addr, len) {
            self.core.watch = a;
            self.core.watch_len = n;
            self.core.watch_limit = 4096;
            self.core.watch_hits.clear();
            self.core.watch_addrs.clear();
            return format!("W {a:x} {n:x}");
        }
        let mut out = String::from("W");
        for ((_, pc, v), a) in self.core.watch_hits.iter().zip(&self.core.watch_addrs) {
            let _ = write!(out, " {pc:x}:{a:x}:{v:x}");
        }
        self.core.watch_hits.clear();
        self.core.watch_addrs.clear();
        out
    }

    /// `E` reply: totals of the external-port chain model and, for `range` (`first`, `count`), the
    /// executed TCBs as `ext/ext_mod:int/int_mod:count:dir` (`i` = external to DM, `o` = DM to external).
    fn epchain_report(&mut self, first: Option<u32>, count: Option<u32>) -> String {
        let Some(e) = self.core.epchain() else {
            return "E off".to_string();
        };
        let mut out = format!(
            "E chains={} tcbs={} words_in={} words_out={} unmapped={}",
            e.chains, e.tcbs, e.words_in, e.words_out, e.unmapped_reads
        );
        if let Some(f) = &e.flash {
            let r = |b| f.reads.get(&b).copied().unwrap_or(0);
            let _ = write!(
                out,
                " flash_b1={} flash_b3={} flash_outside={} latch={} base={}",
                r(0x0400_0000),
                r(0x0C00_0000),
                f.outside,
                f.latch(),
                f.base()
            );
        }
        if let (Some(a), Some(n)) = (first, count) {
            for t in e.log.iter().skip(a as usize).take(n as usize) {
                let _ = write!(
                    out,
                    " {:x}/{:x}:{:x}/{:x}:{:x}:{}",
                    t.ext,
                    t.ext_mod,
                    t.int,
                    t.int_mod,
                    t.count,
                    if t.to_dm { 'i' } else { 'o' }
                );
            }
        }
        out
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
            let (s, ok) = if let Some(sy) = &mut self.sy {
                let r = sy.pass(&mut self.core);
                if let Some(e) = sy.take_error() {
                    self.error.get_or_insert(e);
                }
                r
            } else if self.probing {
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
            if let Some(f) = &mut self.capture {
                let bytes: Vec<u8> = w.iter().flat_map(|b| b.to_le_bytes()).collect();
                if let Err(e) = std::io::Write::write_all(f, &bytes) {
                    self.error.get_or_insert(format!("capture: {e}"));
                }
            }
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
        if self.probing && self.sy.is_none() {
            let list: Vec<String> = self
                .probes
                .iter()
                .zip(&hits)
                .map(|(p, h)| format!("{p:x}:{h}"))
                .collect();
            let _ = write!(out, " hits={}", list.join(","));
        }
        if let Some(sy) = &self.sy {
            let _ = write!(out, " {}", sy.report());
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
    fn bad_p_and_d_lines_are_answered_and_bad_m_lines_are_deferred() {
        let mut l = live(vec![]);
        assert!(l.handle("P x").unwrap().starts_with("ERR "));
        assert!(l.handle("D").unwrap().starts_with("ERR "));
        assert_eq!(l.handle("M zz 1"), None);
        assert!(l.handle("P 1").unwrap().contains("error=bad_command"));
    }

    #[test]
    fn w_g_and_s_watch_writes_dump_registers_and_need_a_sample_chip() {
        let mut l = live(vec![]);
        assert_eq!(
            l.exec("W 0xB0000 2").unwrap(),
            Some("W b0000 2".to_string())
        );
        l.core.pc = 0x12_0002;
        l.core.dm_write(0xB_0001, 7).unwrap();
        l.core.dm_write(0xB_0005, 9).unwrap(); // outside the window
        assert_eq!(l.exec("W").unwrap(), Some("W 120002:b0001:7".to_string()));
        assert_eq!(
            l.exec("W").unwrap(),
            Some("W".to_string()),
            "hits are cleared"
        );
        let g = l.exec("G").unwrap().unwrap();
        assert!(g.starts_with("G pc=120002 r=") && g.contains(" l="), "{g}");
        assert!(l.handle("S").unwrap().starts_with("ERR"), "no sample chip");
    }

    #[test]
    fn l_f_and_e_load_place_and_report_the_flash() {
        let mut l = live(vec![]);
        assert_eq!(l.exec("E").unwrap(), Some("E off".to_string()));
        assert!(l.handle("F 3").unwrap().starts_with("ERR"), "no image yet");
        let p = std::env::temp_dir().join(format!("live_cmd_{}.bin", std::process::id()));
        std::fs::write(&p, [0u8; 64]).unwrap();
        let r = l.exec(&format!("L {}", p.display())).unwrap().unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(r, "L words=32");
        assert!(l.exec("F 3").unwrap().unwrap().starts_with("F base=196298"));
        let e = l.exec("E").unwrap().unwrap();
        assert!(
            e.contains("flash_b1=0 flash_b3=0") && e.contains("base=196298"),
            "{e}"
        );
        assert!(l.handle("L no/such/file").unwrap().starts_with("ERR"));
    }
}
