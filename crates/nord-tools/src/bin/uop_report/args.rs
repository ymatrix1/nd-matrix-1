//! The command line of `uop-report`: [`parse_args`] turns it into [`Args`].
use nord_sharc::runner::parse_number;
use nord_sharc::runner::pass_sched::Engine;
use nord_sharc::runner::uop_pass::{engine_named, uop_config, RangeMode};

pub const USAGE: &str = "usage: uop-report [--passes N] [--warmup N] [--label TEXT] \
[--engine uop|uop-checked|uop-native] [--uop-families BITS] [--ranges extent|probe] [--image FILE --preset NAME [--msgs FILE]]";

/// Where the core comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    /// The synthetic poll program.
    Synthetic,
    /// The user's own raw core SRAM image, its preset name and an optional message file.
    Firmware {
        image: String,
        preset: String,
        msgs: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub source: Source,
    pub passes: u32,
    pub warmup: u32,
    pub engine: Engine,
    pub engine_name: String,
    pub families: u32,
    pub ranges: RangeMode,
    pub label: Option<String>,
}

/// The arguments of the command line, or the reason they are not usable.
pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        source: Source::Synthetic,
        passes: 100,
        warmup: 2,
        engine: Engine::Uop,
        engine_name: "uop".into(),
        families: 0,
        ranges: RangeMode::Extent,
        label: None,
    };
    let (mut image, mut preset, mut msgs) = (None, None, None);
    let mut args = args;
    while let Some(flag) = args.next() {
        let mut val = || args.next().ok_or(format!("{flag} needs a value\n{USAGE}"));
        let count = |v: String, what: &str| v.parse::<u32>().map_err(|_| format!("bad {what} {v}"));
        match flag.as_str() {
            "--passes" => a.passes = count(val()?, "--passes")?,
            "--warmup" => a.warmup = count(val()?, "--warmup")?,
            "--label" => a.label = Some(val()?),
            "--engine" => {
                let name = val()?;
                match engine_named(&name) {
                    Some(e) if uop_config(e, 0).is_some() => (a.engine, a.engine_name) = (e, name),
                    _ => {
                        return Err(format!(
                            "--engine {name:?}: want uop, uop-checked or uop-native"
                        ))
                    }
                }
            }
            "--uop-families" => {
                let v = val()?;
                a.families = parse_number(&v).ok_or(format!("bad --uop-families {v}"))?;
            }
            "--ranges" => {
                let v = val()?;
                a.ranges =
                    RangeMode::named(&v).ok_or(format!("--ranges {v:?}: want extent or probe"))?;
            }
            "--image" => image = Some(val()?),
            "--preset" => preset = Some(val()?),
            "--msgs" => msgs = Some(val()?),
            _ => return Err(USAGE.into()),
        }
    }
    a.source = match (image, preset, msgs) {
        (None, None, None) => Source::Synthetic,
        (Some(image), Some(preset), msgs) => Source::Firmware {
            image,
            preset,
            msgs,
        },
        _ => {
            return Err(format!(
                "--image and --preset go together; --msgs needs both\n{USAGE}"
            ))
        }
    };
    Ok(a)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn arguments_default_parse_and_reject_what_is_not_usable() {
        let d = parse_args(args(&[])).unwrap();
        assert_eq!(
            (d.source, d.passes, d.warmup, d.engine, d.families, d.label),
            (Source::Synthetic, 100, 2, Engine::Uop, 0, None)
        );
        assert_eq!(d.ranges, RangeMode::Extent);
        assert_eq!(
            parse_args(args(&["--ranges", "probe"])).unwrap().ranges,
            RangeMode::Probe
        );
        let a = parse_args(args(&[
            "--passes",
            "50",
            "--warmup",
            "0",
            "--label",
            "x",
            "--engine",
            "uop-checked",
            "--uop-families",
            "0x6",
            "--image",
            "i.bin",
            "--preset",
            "org",
            "--msgs",
            "m.txt",
        ]))
        .unwrap();
        assert_eq!(
            (a.passes, a.warmup, a.engine, a.families),
            (50, 0, Engine::UopChecked, 6)
        );
        assert_eq!(a.label.as_deref(), Some("x"));
        assert_eq!(
            a.source,
            Source::Firmware {
                image: "i.bin".into(),
                preset: "org".into(),
                msgs: Some("m.txt".into())
            }
        );
        for bad in [
            &["--passes", "many"][..],
            &["--passes"],
            &["--bogus"],
            &["--engine", "interp"], // not a uop engine: nothing to report
            &["--engine", "aot"],
            &["--engine", "turbo"],
            &["--uop-families", "x"],
            &["--ranges", "all"],
            &["--ranges"],
            &["--image", "i.bin"], // needs a preset
            &["--preset", "org"],
            &["--msgs", "m.txt"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_engine_name_is_kept_for_the_label() {
        let a = parse_args(args(&["--engine", "uop-native"])).unwrap();
        assert_eq!(
            (a.engine, a.engine_name.as_str()),
            (Engine::UopNative, "uop-native")
        );
        assert!(parse_args(args(&["--engine"]))
            .unwrap_err()
            .contains("needs a value"));
    }
}
