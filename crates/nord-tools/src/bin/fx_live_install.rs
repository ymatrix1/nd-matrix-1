//! `fx_live_install SLOT TYPE [RATE AMOUNT]`: prints the oracle install of a Mod 1 / Mod 2 port
//! (`nord_core::fx::live_install`) as `pc 0x.. i1 0x..` followed by `0xADDR 0xWORD` lines, the
//! input of `tools/fx_vs_oracle.py`. Words only (derived from the port tables), no firmware data.
use nord_core::fx::live_install;

/// Renders the install of `args = [slot, type, rate?, amount?]` as text.
fn render(args: &[String]) -> Result<String, String> {
    let num = |i: usize, d: u32| -> Result<u32, String> {
        args.get(i).map_or(Ok(d), |s| {
            s.parse().map_err(|_| format!("bad number {s:?}"))
        })
    };
    let (slot, ty) = (num(0, u32::MAX)?, num(1, u32::MAX)?);
    let li = live_install(slot as u8, ty as u8, num(2, 64)?, num(3, 64)?)
        .ok_or_else(|| format!("no port export for slot {slot} type {ty}"))?;
    let mut out = format!("pc {:#x} i1 {:#x}\n", li.pc, li.i1);
    for (a, w) in li.words {
        out.push_str(&format!("{a:#x} {w:#010x}\n"));
    }
    Ok(out)
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match render(&args) {
        Ok(s) => {
            print!("{s}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn prints_the_header_and_word_lines() {
        let s = render(&a(&["2", "4", "64", "64"])).unwrap();
        assert!(s.starts_with("pc 0x123798 i1 0xb2040\n"));
        assert!(s.lines().count() > 0x400);
        assert!(s.lines().skip(1).all(|l| l.split(' ').count() == 2));
    }

    #[test]
    fn rejects_unwired_and_malformed_requests() {
        assert!(render(&a(&["1", "12"])).is_err());
        assert!(render(&a(&["2", "x"])).is_err());
        assert!(render(&[]).is_err());
    }
}
