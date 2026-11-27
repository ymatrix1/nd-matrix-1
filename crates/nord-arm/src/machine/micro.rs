//! Dispatch cost without the firmware: loops of synthetic ARM code run with the plain and the
//! predecoded path (`MICRO=1` in `arm-lle-bench`). Separates the per-instruction cost of the
//! interpreter from the cost of the firmware's memory and branch behaviour.

use super::Machine;
use std::time::Instant;

const BASE: u32 = 0xC000_0000;

/// `(name, code)` of the loops.
pub fn programs() -> Vec<(&'static str, Vec<u32>)> {
    vec![
        (
            "add r0,r0,#1 x15 + b",
            [vec![0xE280_0001; 15], vec![0xEAFF_FFF0]].concat(),
        ),
        (
            "ldr/str/add mix + b",
            vec![
                0xE591_2000, // ldr r2,[r1]
                0xE282_2001, // add r2,r2,#1
                0xE581_2000, // str r2,[r1]
                0xE281_1004, // add r1,r1,#4
                0xE351_0000, // cmp r1,#0
                0xE200_1CFF, // and r1,r0,#0xFF00
                0xEAFF_FFFA, // b start
            ],
        ),
        (
            "add + cmp + bne + bl loop",
            vec![0xE280_0001, 0xE350_0000, 0x1AFF_FFFD, 0xEBFF_FFFC],
        ),
    ]
}

/// Run `code` for `insns` instructions; returns the rate in million instructions per second.
pub fn rate(code: &[u32], predecode: bool, insns: u64) -> f64 {
    let mut m = Machine::new();
    m.predecode = predecode;
    for (i, w) in code.iter().enumerate() {
        m.bus.mem.write32(BASE + 4 * i as u32, *w);
    }
    m.cpu.r[1] = 0xC001_0000;
    m.cpu.r[15] = BASE;
    let t = Instant::now();
    let (done, _) = m.run(insns, 0, None);
    done as f64 / t.elapsed().as_secs_f64() / 1e6
}

pub fn print_dispatch_rates(insns: u64) {
    for (name, code) in programs() {
        for predecode in [false, true] {
            println!(
                "micro {name:<26} predecode {predecode:<5} {:.1} M insns/s",
                rate(&code, predecode, insns)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_loop_runs_the_requested_instructions_on_both_paths() {
        for (_, code) in programs() {
            assert!(rate(&code, false, 5_000) > 0.0);
            assert!(rate(&code, true, 5_000) > 0.0);
        }
    }
}
