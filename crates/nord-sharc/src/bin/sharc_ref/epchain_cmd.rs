//! `epchain [path=FILE] [base=ADDR]`: switches on the external-port DMA chain model
//! (`nord_sharc::core::periph::epchain`, D141) for the SY1SY2 image and prints its totals.
//!
//! ```text
//! epchain                      chains run against an empty external memory (reads give 0)
//! epchain path=F base=0x0      also loads the little-endian 32-bit words of F at external word
//!                              address BASE, so the ISR's data TCBs read real packed samples
//! ```
//!
//! Provenance: the file only carries data the user supplies locally; the model and register
//! facts are tagged in `epchain.rs`. The report lists totals and the distinct transfer shapes
//! (direction, count, internal and external ranges), never data words.
use crate::exec::Runner;
use crate::script::{parse_number, Cmd};
use crate::signal::Opts;
use nord_sharc::core::periph::epchain::PITCH_WINDOW;
use nord_sharc::core::Core;
use std::collections::BTreeMap;

/// Largest external image `path=` accepts (ASSUMED: 64 MiB is enough for a slice of one sample;
/// the model keeps one map entry per word).
const MAX_IMAGE_BYTES: u64 = 64 << 20;

/// Transfers listed verbatim at the start of the report.
const FIRST: usize = 6;

/// Parses the command; `None` when `cmd` is not `"epchain"`.
pub fn parse(cmd: &str, opts: &Opts) -> Option<Result<Cmd, String>> {
    if cmd != "epchain" {
        return None;
    }
    let base = match opts.get("base") {
        None => 0,
        Some(s) => match parse_number(s).and_then(|n| u32::try_from(n).ok()) {
            Some(n) => n,
            None => return Some(Err(format!("epchain: bad base {s:?}"))),
        },
    };
    Some(Ok(Cmd::EpChain {
        path: opts.get("path").cloned(),
        base,
    }))
}

impl Runner {
    /// Enables the model and loads the optional external image.
    pub fn enable_epchain(&mut self, path: Option<&str>, base: u32) -> Result<(), String> {
        self.core.enable_epchain();
        let Some(path) = path else { return Ok(()) };
        let len = std::fs::metadata(path)
            .map_err(|e| format!("epchain: {path}: {e}"))?
            .len();
        if len > MAX_IMAGE_BYTES {
            return Err(format!(
                "epchain: {path}: {len} bytes exceeds the {MAX_IMAGE_BYTES} byte cap, cut a slice"
            ));
        }
        if len % 4 != 0 {
            return Err(format!(
                "epchain: {path}: {len} bytes is not a multiple of 4"
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("epchain: {path}: {e}"))?;
        let words: Vec<u32> = (0..bytes.len() / 4)
            .map(|i| {
                let w = [
                    bytes[4 * i],
                    bytes[4 * i + 1],
                    bytes[4 * i + 2],
                    bytes[4 * i + 3],
                ];
                u32::from_le_bytes(w)
            })
            .collect();
        if let Some(e) = self.core.epchain() {
            e.load_words(base, &words);
        }
        Ok(())
    }
}

/// Count and internal / external address range of one transfer shape.
struct Span {
    n: u64,
    ilo: u32,
    ihi: u32,
    elo: u32,
    ehi: u32,
}

/// Text appended to the run output: totals and one line per distinct transfer shape.
pub fn report(core: &mut Core) -> String {
    let Some(e) = core.epchain() else {
        return String::new();
    };
    let mut shapes: BTreeMap<(bool, u32, u32, u32), Span> = BTreeMap::new();
    for t in &e.log {
        let s = shapes
            .entry((t.to_dm, t.count, t.ext_mod, t.int_mod))
            .or_insert(Span {
                n: 0,
                ilo: u32::MAX,
                ihi: 0,
                elo: u32::MAX,
                ehi: 0,
            });
        s.n += 1;
        s.ilo = s.ilo.min(t.int);
        s.ihi = s.ihi.max(t.int);
        s.elo = s.elo.min(t.ext);
        s.ehi = s.ehi.max(t.ext);
    }
    let mut out = format!(
        "\nepchain chains={} tcbs={} words_in={} words_out={} unmapped_reads={} aborted={} window_hits={} window=0x{:X}..0x{:X}",
        e.chains, e.tcbs, e.words_in, e.words_out, e.unmapped_reads, e.aborted, e.window_hits,
        PITCH_WINDOW.0, PITCH_WINDOW.1
    );
    for t in e.log.iter().take(FIRST) {
        out.push_str(&format!(
            "\nepchain first tcb=0x{:X} dir={} ext=0x{:X}+{} int=0x{:X}+{} count={}",
            t.tcb,
            if t.to_dm { "ext->dm" } else { "dm->ext" },
            t.ext,
            t.ext_mod,
            t.int,
            t.int_mod,
            t.count
        ));
    }
    for (
        (to_dm, count, em, im),
        Span {
            n,
            ilo,
            ihi,
            elo,
            ehi,
        },
    ) in shapes
    {
        out.push_str(&format!(
            "\nepchain shape dir={} count={count} ext_mod={em} int_mod={im} n={n} int=0x{ilo:X}..0x{ihi:X} ext=0x{elo:X}..0x{ehi:X}",
            if to_dm { "ext->dm" } else { "dm->ext" }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_path_and_base_and_ignores_other_commands() {
        let mut o = Opts::default();
        assert!(parse("stream", &o).is_none());
        assert!(matches!(
            parse("epchain", &o),
            Some(Ok(Cmd::EpChain {
                path: None,
                base: 0
            }))
        ));
        o.insert("path".into(), "x.bin".into());
        o.insert("base".into(), "0x40".into());
        match parse("epchain", &o) {
            Some(Ok(Cmd::EpChain { path, base })) => {
                assert_eq!((path.as_deref(), base), (Some("x.bin"), 0x40));
            }
            other => panic!("{other:?}"),
        }
        o.insert("base".into(), "zz".into());
        assert!(matches!(parse("epchain", &o), Some(Err(_))));
    }

    #[test]
    fn report_is_empty_when_the_model_is_off_and_lists_shapes_when_on() {
        let mut c = Core::new(&[]);
        assert_eq!(report(&mut c), "");
        c.enable_epchain();
        assert!(report(&mut c).contains("epchain chains=0 tcbs=0"));
    }

    #[test]
    fn the_image_loader_rejects_odd_sizes_and_loads_words_little_endian() {
        let dir = std::env::temp_dir().join("sharc_ref_epchain_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let (odd, good) = (dir.join("odd.bin"), dir.join("good.bin"));
        std::fs::write(&odd, [1u8, 2, 3]).unwrap();
        std::fs::write(&good, [1u8, 0, 0, 0, 2, 0, 0, 0]).unwrap();
        let mut r = Runner::new(&[], false);
        assert!(r.enable_epchain(Some(odd.to_str().unwrap()), 0).is_err());
        assert!(r.enable_epchain(Some("no/such/file.bin"), 0).is_err());
        r.enable_epchain(Some(good.to_str().unwrap()), 0x10)
            .unwrap();
        let e = r.core.epchain().unwrap();
        assert_eq!((e.ext.get(&0x10), e.ext.get(&0x11)), (Some(&1), Some(&2)));
    }
}
