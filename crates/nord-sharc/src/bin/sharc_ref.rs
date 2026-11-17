//! `sharc-ref IMAGE SCENARIO [--out PREFIX] [--wav] [--lenient] [--watch LO:LEN[:LIMIT]]`: oracle reference render.
//!
//! Loads a flat SRAM image (`python tools/ns4_dsp_raw.py IMG --sram DIR`), executes a scenario
//! file (data pokes, test-signal fills, voice records, handler runs, captures; format in
//! `script.rs` and `docs/research/oracle-vs-engine.md`) on the SHARC core and writes the captured
//! series to `PREFIX.csv` (with a metadata header) and, with `--wav`, `PREFIX.wav`. Without
//! `--out` the CSV goes to standard output. `tools/ns4_oracle_compare.py` compares the dump with
//! a reference render of the Rust engine. `--watch LO:LEN[:LIMIT]` appends a log of every data
//! write into `[LO, LO+LEN)` (core step, pc, address, value; scenario pokes show the pc of the
//! previous store) to the output message.
#[path = "sharc_ref/epchain_cmd.rs"]
mod epchain_cmd;
#[path = "sharc_ref/exec.rs"]
mod exec;
#[path = "sharc_ref/isr.rs"]
mod isr;
#[path = "sharc_ref/out.rs"]
mod out;
#[path = "sharc_ref/pcm_ring.rs"]
mod pcm_ring;
#[path = "sharc_ref/refill.rs"]
mod refill;
#[path = "sharc_ref/script.rs"]
mod script;
#[path = "sharc_ref/signal.rs"]
mod signal;
#[path = "sharc_ref/stream.rs"]
mod stream;
#[path = "sharc_ref/voice.rs"]
mod voice;

use std::process::ExitCode;

fn run(args: &[String]) -> Result<String, String> {
    let mut positional = Vec::new();
    let mut prefix: Option<String> = None;
    let mut wav = false;
    let mut lenient = false;
    let mut watch: Option<(u32, u32, usize)> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => prefix = Some(it.next().ok_or("--out needs PREFIX")?.clone()),
            "--wav" => wav = true,
            "--lenient" => lenient = true,
            "--watch" => {
                watch = Some(parse_watch(
                    it.next().ok_or("--watch needs LO:LEN[:LIMIT]")?,
                )?)
            }
            _ => positional.push(a.clone()),
        }
    }
    let [image, scenario] = positional.as_slice() else {
        return Err("usage: sharc-ref IMAGE SCENARIO [--out PREFIX] [--wav] [--lenient] [--watch LO:LEN[:LIMIT]]".into());
    };
    let data = std::fs::read(image).map_err(|e| format!("{image}: {e}"))?;
    let text = std::fs::read_to_string(scenario).map_err(|e| format!("{scenario}: {e}"))?;
    let cmds = script::parse_script(&text)?;
    let mut runner = exec::Runner::new(&data, lenient);
    if let Some((lo, len, limit)) = watch {
        (
            runner.core.watch,
            runner.core.watch_len,
            runner.core.watch_limit,
        ) = (lo, len, limit);
    }
    runner.exec(&cmds)?;
    let mut log = watch_log(&runner.core);
    log.push_str(&epchain_cmd::report(&mut runner.core));
    let render = &runner.render;
    let extra = vec![
        ("tool".to_string(), "sharc-ref".to_string()),
        ("scenario".to_string(), scenario.clone()),
        ("runs".to_string(), render.run_steps.len().to_string()),
        (
            "steps_per_run_max".to_string(),
            render.run_steps.iter().max().unwrap_or(&0).to_string(),
        ),
        ("core_steps".to_string(), runner.core.steps.to_string()),
    ];
    let csv = out::to_csv(render, &extra);
    let rate = render
        .meta
        .iter()
        .find(|m| m.0 == "rate")
        .and_then(|m| m.1.parse().ok())
        .unwrap_or(48_000);
    match prefix {
        None => Ok(format!("{csv}{log}")),
        Some(p) => {
            std::fs::write(format!("{p}.csv"), &csv).map_err(|e| e.to_string())?;
            let mut msg = format!("wrote {p}.csv ({} series)", render.series.len());
            if wav {
                std::fs::write(format!("{p}.wav"), out::to_wav(render, rate)?)
                    .map_err(|e| e.to_string())?;
                msg.push_str(&format!(", {p}.wav"));
            }
            msg.push_str(&log);
            Ok(msg)
        }
    }
}

/// `LO:LEN[:LIMIT]` (numbers decimal or `0x` hex; limit defaults to 100000 hits).
fn parse_watch(spec: &str) -> Result<(u32, u32, usize), String> {
    let num = |t: &str| {
        let t = t.trim();
        match t.strip_prefix("0x") {
            Some(h) => u32::from_str_radix(h, 16),
            None => t.parse::<u32>(),
        }
        .map_err(|_| format!("bad --watch number {t:?}"))
    };
    let mut f = spec.split(':');
    let lo = num(f.next().unwrap_or(""))?;
    let len = num(f.next().ok_or("--watch needs LO:LEN[:LIMIT]")?)?;
    let limit = f.next().map_or(Ok(100_000), num)? as usize;
    Ok((lo, len, limit))
}

/// One line per logged write: `watch step=.. pc=0x.. addr=0x.. value=0x..`.
fn watch_log(core: &nord_sharc::core::Core) -> String {
    let mut out = String::new();
    for ((step, pc, value), addr) in core.watch_hits.iter().zip(&core.watch_addrs) {
        out.push_str(&format!(
            "
watch step={step} pc=0x{pc:X} addr=0x{addr:X} value=0x{value:08X}"
        ));
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every committed scenario parses (loops included) and uses only known commands.
    #[test]
    fn committed_scenarios_parse() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/oracle_scenarios");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "scn") {
                let text = std::fs::read_to_string(&path).unwrap();
                let cmds = script::parse_script(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"));
                assert!(!cmds.is_empty(), "{path:?}");
                seen += 1;
            }
        }
        assert!(seen >= 4, "expected the Stage B scenarios, found {seen}");
    }

    #[test]
    fn watch_logs_scenario_writes_into_the_window() {
        assert_eq!(
            parse_watch("0x9354A:0x46").unwrap(),
            (0x9354A, 0x46, 100_000)
        );
        assert_eq!(parse_watch("10:2:5").unwrap(), (10, 2, 5));
        assert!(parse_watch("zz:1").is_err() && parse_watch("1").is_err());
        let dir = std::env::temp_dir().join("nord_sharc_ref_watch");
        std::fs::create_dir_all(&dir).unwrap();
        let (image, scen) = (dir.join("img.bin"), dir.join("s.txt"));
        std::fs::write(&image, vec![0u8; 64]).unwrap();
        std::fs::write(
            &scen,
            "poke 0xA0000 7
poke 0xA0005 9
capture L 0xA0000 1
",
        )
        .unwrap();
        let args = [
            image.to_string_lossy().to_string(),
            scen.to_string_lossy().to_string(),
            "--watch".into(),
            "0xA0000:2".into(),
        ];
        let text = run(&args).unwrap();
        assert!(
            text.contains("watch step=0 pc=0x0 addr=0xA0000 value=0x00000007"),
            "{text}"
        );
        assert!(!text.contains("0xA0005"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writes_csv_and_wav_for_a_poke_scenario() {
        let dir = std::env::temp_dir().join("nord_sharc_ref_test");
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("img.bin");
        std::fs::write(&image, vec![0u8; 64]).unwrap();
        let scen = dir.join("s.txt");
        std::fs::write(
            &scen,
            "meta rate 44100\npoke 0xA0000 f0.5\npoke 0xA0001 f-0.25\ncapture L 0xA0000 1\ncapture R 0xA0001 1\n",
        )
        .unwrap();
        let (i, s) = (
            image.to_string_lossy().to_string(),
            scen.to_string_lossy().to_string(),
        );
        let csv = run(&[i.clone(), s.clone()]).unwrap();
        assert!(csv.contains("index,L,R\n0,5e-1,-2.5e-1"), "{csv}");
        let prefix = dir.join("o").to_string_lossy().to_string();
        let msg = run(&[i.clone(), s, "--out".into(), prefix.clone(), "--wav".into()]).unwrap();
        assert!(msg.contains("o.wav"), "{msg}");
        let wav = std::fs::read(format!("{prefix}.wav")).unwrap();
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 44100);
        assert!(run(&[i]).is_err());
        assert!(run(&["x".into(), "y".into()]).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
