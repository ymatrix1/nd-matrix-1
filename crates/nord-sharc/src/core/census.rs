//! Runs the cases of the firmware-instruction census diff on a [`Core`]
//! (`docs/research/sharc-seq/gen_census_probe.py`, `sim_batch.py`).
//!
//! The fixture holds the probe's machine code and the symbols `_slot`, `_st`, `_res`, `_scr`. A case
//! line is `PARCELS S0 .. Sn` (hex): the instruction's 16-bit parcels poked into the slot and the
//! state table written to `_st`; a `scr` line gives the 512-word scratch table every case starts
//! with. Each case starts on a fresh core; the output line holds the dumped state (`_res`, the words
//! between `_res` and `_scr`) followed by the scratch table, as hex words, or `FAULT message`.
use super::probe_fixture::Fixture;
use super::Core;

const SLOT_PARCELS: usize = 10;
const SCR_WORDS: usize = 512;
const NOP: u16 = 0x0001;
const MAX_STEPS: usize = 20_000;

fn words(tokens: &[&str]) -> Vec<u32> {
    tokens
        .iter()
        .map(|t| u32::from_str_radix(t, 16).unwrap_or(0))
        .collect()
}

/// Memory set-up shared by all cases of a file (`code`, `dm` and `watch` header lines).
#[derive(Default)]
struct Setup {
    code: Vec<(u32, Vec<u16>)>,
    dm: Vec<(u32, Vec<u32>)>,
    watch: Vec<(u32, usize)>,
    /// A `plus` header line: run on a SHARC+ core (`Core::sharc_plus`).
    plus: bool,
}

/// Which executor runs a census case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Engine {
    /// [`Core::step`] (the oracle).
    Interp,
    /// Level-2 Cranelift JIT ([`Core::run_jit`]) for the same number of steps the interpreter took.
    #[cfg(feature = "jit")]
    Jit,
    /// The uop engine (`core/uop`, D260) armed on the probe code and run for the steps the
    /// interpreter took. Steps it does not lower fall back to [`Core::step`].
    Uop,
    /// [`Engine::Uop`], and the dumped state must equal the interpreter's (a mismatch is an error).
    UopChecked,
}

fn case_run(
    engine: Engine,
    f: &Fixture,
    scr: &[u32],
    setup: &Setup,
    parcels: &str,
    state: &[u32],
    chunks: &[(u32, Vec<u32>)],
) -> Result<(String, u64), String> {
    let sym = |n: &str| f.sym(n).ok_or_else(|| format!("fixture lacks sym {n}"));
    let (slot, st, res, scr_a) = (sym("_slot")?, sym("_st")?, sym("_res")?, sym("_scr")?);
    let mut c = Core::new(&[]);
    c.sharc_plus = setup.plus;
    f.load(&mut c)?;
    for (addr, ps) in &setup.code {
        for (i, p) in ps.iter().enumerate() {
            let o = (*addr as usize + i) * 2;
            c.sram[o..o + 2].copy_from_slice(&p.to_le_bytes());
        }
    }
    for (addr, ws) in setup.dm.iter().chain(chunks) {
        for (i, w) in ws.iter().enumerate() {
            c.dm_write(addr + i as u32, *w)
                .map_err(|e| format!("dm write {e:x}"))?;
        }
    }
    let mut ps: Vec<u16> = (0..parcels.len() / 4)
        .map(|i| u16::from_str_radix(&parcels[4 * i..4 * i + 4], 16).unwrap_or(NOP))
        .collect();
    ps.resize(SLOT_PARCELS, NOP);
    for (i, p) in ps.iter().enumerate() {
        let o = (slot as usize + i) * 2;
        c.sram[o..o + 2].copy_from_slice(&p.to_le_bytes());
    }
    for (i, w) in state.iter().enumerate() {
        c.dm_write(st + i as u32, *w)
            .map_err(|e| format!("st write {e:x}"))?;
    }
    for (i, w) in scr.iter().enumerate() {
        c.dm_write(scr_a + i as u32, *w)
            .map_err(|e| format!("scr write {e:x}"))?;
    }
    c.pc = f.entry;
    let mut steps = 0;
    // debugging aid: `CENSUS_STOP=hex` prints R, S, I, M when the pc first reaches that address
    if let Some(stop) = std::env::var("CENSUS_STOP")
        .ok()
        .and_then(|v| u32::from_str_radix(&v, 16).ok())
    {
        let mut hits: usize = std::env::var("CENSUS_HIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        while c.pc != f.exit && steps <= MAX_STEPS {
            if c.pc == stop {
                hits -= 1;
                if hits == 0 {
                    break;
                }
            }
            c.step().map_err(|e| format!("pc {:x}: {e:?}", c.pc))?;
            steps += 1;
        }
        let regs = c.r.iter().chain(&c.s).chain(&c.i).chain(&c.m);
        return Ok((
            format!(
                "{:x} {}",
                c.pc,
                regs.map(|w| format!("{w:08X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            steps as u64,
        ));
    }
    if engine == Engine::UopChecked {
        let (got, n) = case_run(Engine::Uop, f, scr, setup, parcels, state, chunks)?;
        let (want, _) = case_run(Engine::Interp, f, scr, setup, parcels, state, chunks)?;
        if got != want {
            return Err(format!(
                "uop state differs from the interpreter: {got} vs {want}"
            ));
        }
        return Ok((got, n));
    }
    if engine == Engine::Uop {
        // the interpreter run of the same case gives the step count; the uop engine then runs
        // exactly that. A range the engine rejects leaves it off, so the case runs interpreted.
        let n = case_run(Engine::Interp, f, scr, setup, parcels, state, chunks)?.1;
        let _ = c.enable_uop(super::UopConfig::default(), &[(f.entry, f.exit)]);
        if let Some(fault) = c.run_uop(n) {
            return Err(format!("uop fault {fault:?}"));
        }
        if c.pc != f.exit {
            return Err(format!("uop stopped at pc {:x}, not the exit", c.pc));
        }
    }
    #[cfg(feature = "jit")]
    if engine == Engine::Jit {
        // the interpreter run of the same case gives the step count; the JIT then runs exactly that
        let n = case_run(Engine::Interp, f, scr, setup, parcels, state, chunks)?.1;
        if let Some(fault) = c.run_jit(n) {
            return Err(format!("jit fault {fault:?}"));
        }
        if c.pc != f.exit {
            return Err(format!("jit stopped at pc {:x}, not the exit", c.pc));
        }
    }
    while engine == Engine::Interp && c.pc != f.exit {
        c.step()
            .map_err(|e| format!("pc {:x} after {steps} steps: {e:?}", c.pc))?;
        steps += 1;
        if steps > MAX_STEPS {
            return Err(format!("no exit, pc {:x}", c.pc));
        }
    }
    let mut out = Vec::new();
    let watched = setup.watch.iter().flat_map(|(a, n)| *a..*a + *n as u32);
    for a in (res..scr_a)
        .chain(scr_a..scr_a + SCR_WORDS as u32)
        .chain(watched)
    {
        let w = c.dm_read(a).map_err(|e| format!("read {e:x}"))?;
        out.push(format!("{w:08X}"));
    }
    Ok((out.join(" "), steps as u64))
}

fn one_case(
    engine: Engine,
    f: &Fixture,
    scr: &[u32],
    setup: &Setup,
    parcels: &str,
    state: &[u32],
    chunks: &[(u32, Vec<u32>)],
) -> Result<String, String> {
    case_run(engine, f, scr, setup, parcels, state, chunks).map(|r| r.0)
}

/// Compact form of an output line for the committed regression fixtures: the dumped state words,
/// then ` @index=value` for each scratch word that differs from the table the case started with.
/// `FAULT` and `HALT` lines stay as they are.
pub fn compact(line: &str, scr: &[u32], n_state: usize) -> String {
    if line.starts_with("FAULT") || line.starts_with("HALT") {
        return line.to_string();
    }
    let w = words(&line.split_whitespace().collect::<Vec<_>>());
    let mut out: Vec<String> = w.iter().take(n_state).map(|x| format!("{x:08X}")).collect();
    for (i, x) in w.iter().skip(n_state).enumerate() {
        if scr.get(i) != Some(x) {
            out.push(format!("@{i}={x:08X}"));
        }
    }
    out.join(" ")
}

/// The scratch table of a cases file (its `scr` line).
pub fn scratch_of(cases: &str) -> Vec<u32> {
    cases
        .lines()
        .find_map(|l| l.strip_prefix("scr "))
        .map(|r| words(&r.split_whitespace().collect::<Vec<_>>()))
        .unwrap_or_default()
}

/// Runs every case of `cases` and returns one output line per case.
pub fn run_cases(fixture: &str, cases: &str) -> Result<String, String> {
    run_cases_on(Engine::Interp, fixture, cases)
}

/// [`run_cases`] on the chosen executor.
pub fn run_cases_on(engine: Engine, fixture: &str, cases: &str) -> Result<String, String> {
    let f = Fixture::parse(fixture);
    let mut scr = vec![0u32; SCR_WORDS];
    let mut setup = Setup::default();
    let mut out = String::new();
    for line in cases.lines() {
        let mut parts = line.split('|');
        let head: Vec<&str> = parts.next().unwrap_or("").split_whitespace().collect();
        let chunks: Vec<(u32, Vec<u32>)> = parts
            .filter_map(|c| {
                let t: Vec<&str> = c.split_whitespace().collect();
                let (a, rest) = t.split_first()?;
                Some((u32::from_str_radix(a, 16).ok()?, words(rest)))
            })
            .collect();
        match head.as_slice() {
            [] => {}
            ["scr", rest @ ..] => scr = words(rest),
            ["plus"] => setup.plus = true,
            ["code", a, hex] => {
                let ps = (0..hex.len() / 4)
                    .map(|i| u16::from_str_radix(&hex[4 * i..4 * i + 4], 16).unwrap_or(NOP))
                    .collect();
                setup
                    .code
                    .push((u32::from_str_radix(a, 16).unwrap_or(0), ps));
            }
            ["dm", a, rest @ ..] => setup
                .dm
                .push((u32::from_str_radix(a, 16).unwrap_or(0), words(rest))),
            ["watch", a, n] => setup.watch.push((
                u32::from_str_radix(a, 16).unwrap_or(0),
                n.parse().unwrap_or(0),
            )),
            [parcels, rest @ ..] => {
                let line = one_case(engine, &f, &scr, &setup, parcels, &words(rest), &chunks)
                    .unwrap_or_else(|e| format!("FAULT {e}"));
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_symbol_is_reported_per_case() {
        let out = run_cases("entry 0\nexit 0\n", "scr 1\n0001 0 0\n").unwrap();
        assert!(out.starts_with("FAULT fixture lacks sym _slot"), "{out}");
    }

    #[test]
    fn words_parse_hex_and_default_to_zero() {
        assert_eq!(words(&["1F", "zz"]), vec![0x1F, 0]);
    }

    #[test]
    fn compact_keeps_state_words_and_changed_scratch_words_only() {
        let scr = [1, 2, 3];
        let line = "0000000A 0000000B 00000001 00000009 00000003";
        assert_eq!(compact(line, &scr, 2), "0000000A 0000000B @1=00000009");
        assert_eq!(compact("FAULT x", &scr, 2), "FAULT x");
        assert_eq!(
            scratch_of(
                "scr 1 2 3
0001 0
"
            ),
            vec![1, 2, 3]
        );
    }
}
