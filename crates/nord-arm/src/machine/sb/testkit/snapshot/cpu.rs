//! OWNER: W1-VGEN. The CPU half of a snapshot: registers, flags, modes, counters and the fault,
//! with a readable diff (registers in hex, the flag nibble as `NZCV` letters).

use crate::cpu::{Cpu, Fault};

/// `NZCV` with a letter in lower case when the flag is clear (`nzcv` nibble `n << 3 | ...`).
pub(crate) fn flags_str(f: u8) -> String {
    ["N", "Z", "C", "V"]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if f >> (3 - i) & 1 != 0 {
                l.to_string()
            } else {
                l.to_lowercase()
            }
        })
        .collect()
}

/// The CPU state every tier must agree on, with a readable diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CpuSnap {
    pub r: [u32; 16],
    pub nzcv: u8,
    pub q: bool,
    pub thumb: bool,
    pub irq_off: bool,
    pub fiq_off: bool,
    pub mode: u32,
    pub mode_changes: u64,
    pub spsr: u32,
    pub cp15_ctrl: u32,
    pub insns: u64,
    pub fault: Option<Fault>,
}

impl CpuSnap {
    pub fn of(cpu: &Cpu, fault: Option<Fault>) -> Self {
        Self {
            r: cpu.r,
            nzcv: cpu.nzcv() as u8,
            q: cpu.q,
            thumb: cpu.thumb,
            irq_off: cpu.irq_off,
            fiq_off: cpu.fiq_off,
            mode: cpu.mode,
            mode_changes: cpu.mode_changes,
            spsr: cpu.spsr,
            cp15_ctrl: cpu.cp15_ctrl,
            insns: cpu.insns,
            fault,
        }
    }

    /// One entry per differing field joined by `; `; `None` when equal.
    pub fn diff(&self, o: &Self) -> Option<String> {
        let mut out = Vec::new();
        for i in 0..16 {
            if self.r[i] != o.r[i] {
                out.push(format!("r{i}: {:#x} != {:#x}", self.r[i], o.r[i]));
            }
        }
        if self.nzcv != o.nzcv {
            out.push(format!(
                "nzcv: {} != {}",
                flags_str(self.nzcv),
                flags_str(o.nzcv)
            ));
        }
        macro_rules! field {
            ($($f:ident),*) => {$(
                if self.$f != o.$f {
                    out.push(format!("{}: {:x?} != {:x?}", stringify!($f), self.$f, o.$f));
                }
            )*};
        }
        field!(
            q,
            thumb,
            irq_off,
            fiq_off,
            mode,
            mode_changes,
            spsr,
            cp15_ctrl,
            insns,
            fault
        );
        (!out.is_empty()).then(|| out.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change of one field, applied to a copy to prove the comparator sees it.
    type Mutation<T> = Box<dyn Fn(&mut T)>;

    #[test]
    fn the_cpu_comparator_reports_every_field_it_claims_to_compare() {
        let base = CpuSnap::of(&Cpu::new(), None);
        assert_eq!(base.diff(&base.clone()), None);
        let mutations: Vec<(&str, Mutation<CpuSnap>)> = vec![
            ("r7", Box::new(|s| s.r[7] ^= 1)),
            ("r15", Box::new(|s| s.r[15] ^= 4)),
            ("nzcv", Box::new(|s| s.nzcv ^= 2)),
            ("q", Box::new(|s| s.q = !s.q)),
            ("thumb", Box::new(|s| s.thumb = !s.thumb)),
            ("irq_off", Box::new(|s| s.irq_off = !s.irq_off)),
            ("fiq_off", Box::new(|s| s.fiq_off = !s.fiq_off)),
            ("mode", Box::new(|s| s.mode ^= 1)),
            ("mode_changes", Box::new(|s| s.mode_changes += 1)),
            ("spsr", Box::new(|s| s.spsr ^= 1)),
            ("cp15_ctrl", Box::new(|s| s.cp15_ctrl ^= 1)),
            ("insns", Box::new(|s| s.insns += 1)),
            ("fault", Box::new(|s| s.fault = Some(Fault::Wfi))),
        ];
        for r in 0..16 {
            let name = format!("r{r}");
            let mut m = base.clone();
            m.r[r] = 0xDEAD_0000 | r as u32;
            let d = base
                .diff(&m)
                .unwrap_or_else(|| panic!("{name} not compared"));
            assert!(d.starts_with(&format!("{name}: 0x0 != 0xdead")), "{d}");
        }
        for (name, mutate) in mutations {
            let mut m = base.clone();
            mutate(&mut m);
            let d = base
                .diff(&m)
                .unwrap_or_else(|| panic!("{name} not compared"));
            assert!(d.contains(name), "{name}: {d}");
        }
    }

    #[test]
    fn the_diff_is_readable_flags_as_letters_registers_in_hex_one_entry_per_field() {
        let a = CpuSnap::of(&Cpu::new(), None);
        let mut b = a.clone();
        b.nzcv = 0b1010;
        b.r[3] = 0x20;
        b.insns = 7;
        assert_eq!(
            a.diff(&b).unwrap(),
            "r3: 0x0 != 0x20; nzcv: nzcv != NzCv; insns: 0 != 7"
        );
        assert_eq!(flags_str(0b0101), "nZcV");
        assert_eq!(flags_str(0), "nzcv");
        assert_eq!(flags_str(15), "NZCV");
    }
}
