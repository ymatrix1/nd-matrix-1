//! Native lowering of the data-memory transfers for the whole-pass AOT (bean `ns4-lle3`): the
//! `DM`/`PM` loads and stores of Types 3, 4, 14, 15 and 16 and the `MODIFY` of Type 7, resolved at
//! generation time into a [`MemXfer`] constant. [`Core::aot_xfer`] is the interpreter's own
//! sequence for these forms (`moves.rs::mem_transfer_with`, `moves_types.rs::exec_move`) with the
//! decoding done once and the data access reduced to the internal-SRAM fast path.
//!
//! Scope, everything else stays on the interpreter (`aot_item`):
//! unconditional forms only (`IF` condition 31), no long-word (`LW`) access, no ISA-mode code.
//!
//! The fast path is exact because the block guard (`Core::aot_ready`) requires no peripheral
//! model, no data watch and no JIT: then `dm_read` of a SRAM word is the flat-image read and
//! `dm_write` the flat-image write plus the decode-cache invalidation, which only matters inside
//! the cached-code extent. Provenance: VERIFIED against the interpreter by
//! `xfer_equals_the_interpreter_on_random_states` (every form, SIMD and scalar) and by the
//! whole-pass differential on the real ORG core 2 pass (`docs/research/sharc-lle-realtime.md`).
use super::aot_native::Kern;
use super::aot_pe::Pe;
use super::decode_cache::{ComputeOp, DecodedOp, MoveOp};
use super::shifter::shift_imm_params;
use super::{sign_extend, Core, Fault};
use crate::visa::{bits, lw_flag, Insn, Kind};
use std::fmt::Write as _;

/// Normal-word data addresses backed by the flat SRAM image (`memory.rs::SRAM_WORDS`).
#[allow(dead_code)] // only the generated AOT module reaches these
const SRAM_LO: u32 = 0x9_0000;
#[allow(dead_code)]
const SRAM_N: u32 = 0x10_0000 - 0x9_0000;

/// How the access address is formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Addr {
    /// No memory access (register forms).
    None,
    /// Absolute address (Type 14a).
    Direct(u32),
    /// `Ii` post-modified by `Mm` (the register number includes the DAG2 offset).
    PostM(u8),
    /// `Ii + Mm`, no write-back, no wrap.
    PreM(u8),
    /// `Ii` post-modified by a signed immediate.
    PostImm(u32),
    /// `Ii + immediate`, no write-back.
    PreImm(u32),
}

/// What moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Data {
    /// Universal register `ureg`, loaded or stored.
    Reg { ureg: u8, write: bool },
    /// An immediate store (Type 16a/b), also written to the next word in SIMD mode.
    Imm(u32),
    /// `MODIFY(Is, Mm)` (Type 7): no memory access; `Id = Is + M` always circular.
    Modify { dst: u8, src: u8 },
    /// `Un = <data>` (Type 17), any universal register.
    LoadImm { ureg: u8, value: u32 },
    /// `Rr <-> Ss` (Type 5, swap form).
    Swap { r: u8, s: u8 },
    /// `Rdst = Rsrc` for any universal registers (Type 5, move form).
    Mov { src: u8, dst: u8 },
    /// `Rn = <shifter op> Rx BY <immediate>` (Type 6n) with the register-form opcode.
    Shift { op: u64, n: u8, x: u8, ry: u32 },
    /// A compute field alone (Type 2) that the register-resident kernels do not model.
    Compute,
}

/// One decode-time resolved transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MemXfer {
    /// Index register used (`0..16`); unused for [`Addr::Direct`].
    pub ii: u8,
    pub addr: Addr,
    pub data: Data,
    pub compute: Option<ComputeOp>,
}

impl MemXfer {
    /// The transfer of the instruction `insn` (instruction word `w`), when it is a form this
    /// module lowers: unconditional, not long-word.
    pub(super) fn from_insn(insn: &Insn, w: u64) -> Option<MemXfer> {
        let kind = insn.kind;
        if lw_flag(kind, w) {
            return None;
        }
        let cond = bits(w, 37, 33);
        let bit = |b| bits(w, b, b) == 1;
        let compute = || ComputeOp::decode(bits(w, 22, 0));
        let bank = |b: u32| if bit(b) { 8u8 } else { 0 };
        Some(match kind {
            Kind::T14a => MemXfer {
                ii: 0,
                addr: Addr::Direct(bits(w, 31, 0) as u32),
                data: Data::Reg {
                    ureg: bits(w, 38, 32) as u8,
                    write: bit(40),
                },
                compute: None,
            },
            Kind::T15a => MemXfer {
                ii: bits(w, 43, 41) as u8 + bank(44),
                addr: Addr::PreImm(bits(w, 31, 0) as u32),
                data: Data::Reg {
                    ureg: bits(w, 38, 32) as u8,
                    write: bit(40),
                },
                compute: None,
            },
            Kind::T15b => MemXfer {
                ii: bits(w, 43, 41) as u8 + bank(37),
                addr: Addr::PreImm(sign_extend(bits(w, 22, 16) as u32, 7)),
                data: Data::Reg {
                    ureg: bits(w, 29, 23) as u8,
                    write: bit(40),
                },
                compute: None,
            },
            Kind::T3a | Kind::T3b if cond == 31 => {
                let b = bank(32);
                let (ii, mi) = (bits(w, 43, 41) as u8 + b, bits(w, 40, 38) as u8 + b);
                MemXfer {
                    ii,
                    addr: if bit(44) {
                        Addr::PostM(mi)
                    } else {
                        Addr::PreM(mi)
                    },
                    data: Data::Reg {
                        ureg: bits(w, 29, 23) as u8,
                        write: bit(31),
                    },
                    compute: (kind == Kind::T3a).then(compute),
                }
            }
            Kind::T3c => MemXfer {
                ii: bits(w, 43, 41) as u8,
                addr: Addr::PostM(bits(w, 40, 38) as u8),
                data: Data::Reg {
                    ureg: bits(w, 35, 32) as u8,
                    write: bit(37),
                },
                compute: None,
            },
            Kind::T4a | Kind::T4b if cond == 31 => {
                let data = sign_extend(bits(w, 32, 27) as u32, 6);
                MemXfer {
                    ii: bits(w, 43, 41) as u8 + bank(40),
                    addr: if bit(38) {
                        Addr::PostImm(data)
                    } else {
                        Addr::PreImm(data)
                    },
                    data: Data::Reg {
                        ureg: bits(w, 26, 23) as u8,
                        write: bit(39),
                    },
                    compute: (kind == Kind::T4a).then(compute),
                }
            }
            Kind::T16a | Kind::T16b => {
                let b = bank(37);
                let value = if kind == Kind::T16a {
                    bits(w, 31, 0) as u32
                } else {
                    sign_extend(bits(w, 31, 16) as u32, 16)
                };
                MemXfer {
                    ii: bits(w, 43, 41) as u8 + b,
                    addr: Addr::PostM(bits(w, 40, 38) as u8 + b),
                    data: Data::Imm(value),
                    compute: None,
                }
            }
            Kind::T7a | Kind::T7b if cond == 31 => {
                let b = bank(38);
                let src = bits(w, 32, 30) as u8 + b;
                MemXfer {
                    ii: src,
                    addr: Addr::PostM(bits(w, 29, 27) as u8 + b),
                    data: Data::Modify {
                        dst: src ^ bits(w, 26, 24) as u8,
                        src,
                    },
                    compute: (kind == Kind::T7a).then(compute),
                }
            }
            _ => return None,
        })
    }

    /// The register-form transfer of an unconditional decoded op, when lowered.
    pub(super) fn from_decoded(op: &DecodedOp, w: u64) -> Option<MemXfer> {
        let none = |data, compute| MemXfer {
            ii: 0,
            addr: Addr::None,
            data,
            compute,
        };
        match *op {
            DecodedOp::Move(MoveOp::LoadImm { ureg, value }) => {
                Some(none(Data::LoadImm { ureg, value }, None))
            }
            DecodedOp::Move(MoveOp::RegSwap {
                r,
                s,
                cond: 31,
                compute,
            }) => Some(none(Data::Swap { r, s }, compute)),
            DecodedOp::Move(MoveOp::RegMove {
                src,
                dst,
                cond: 31,
                compute,
            }) => Some(none(Data::Mov { src, dst }, compute)),
            DecodedOp::ShiftImm if bits(w, 37, 33) == 31 => {
                shift_imm_params(w).map(|(op, n, x, ry)| {
                    none(
                        Data::Shift {
                            op,
                            n: n as u8,
                            x: x as u8,
                            ry,
                        },
                        None,
                    )
                })
            }
            DecodedOp::ComputeCond { cond: 31, op } | DecodedOp::ComputeBoth { op }
                if op != ComputeOp::Invalid =>
            {
                Some(none(Data::Compute, Some(op)))
            }
            _ => None,
        }
    }

    /// Whether executing this transfer may change state the block guard assumes constant.
    pub(super) fn may_change_guard(&self) -> bool {
        match self.data {
            Data::Reg { ureg, write } => write || ureg >= 0x60,
            Data::Imm(_) => true,
            Data::Modify { .. } | Data::Swap { .. } | Data::Shift { .. } | Data::Compute => false,
            Data::LoadImm { ureg, .. } => ureg >= 0x60,
            Data::Mov { dst, .. } => dst >= 0x60,
        }
    }

    /// Rust expression of this constant for the generated module (`MemXfer`, `Addr`, `Data`,
    /// `ComputeOp` in scope).
    pub(super) fn to_rust(self) -> String {
        let mut s = String::new();
        let addr = match self.addr {
            Addr::None => "Addr::None".to_string(),
            Addr::Direct(a) => format!("Addr::Direct(0x{a:x})"),
            Addr::PostM(m) => format!("Addr::PostM({m})"),
            Addr::PreM(m) => format!("Addr::PreM({m})"),
            Addr::PostImm(d) => format!("Addr::PostImm(0x{d:x})"),
            Addr::PreImm(d) => format!("Addr::PreImm(0x{d:x})"),
        };
        let data = match self.data {
            Data::Reg { ureg, write } => format!("Data::Reg {{ ureg: {ureg}, write: {write} }}"),
            Data::Imm(v) => format!("Data::Imm(0x{v:x})"),
            Data::Modify { dst, src } => format!("Data::Modify {{ dst: {dst}, src: {src} }}"),
            Data::LoadImm { ureg, value } => {
                format!("Data::LoadImm {{ ureg: {ureg}, value: 0x{value:x} }}")
            }
            Data::Swap { r, s } => format!("Data::Swap {{ r: {r}, s: {s} }}"),
            Data::Mov { src, dst } => format!("Data::Mov {{ src: {src}, dst: {dst} }}"),
            Data::Shift { op, n, x, ry } => {
                format!("Data::Shift {{ op: 0x{op:x}, n: {n}, x: {x}, ry: 0x{ry:x} }}")
            }
            Data::Compute => "Data::Compute".to_string(),
        };
        let compute = match self.compute {
            Some(c) => format!("Some(ComputeOp::{c:?})"),
            None => "None".to_string(),
        };
        let _ = write!(
            s,
            "MemXfer {{ ii: {}, addr: {addr}, data: {data}, compute: {compute} }}",
            self.ii
        );
        s
    }
}

#[allow(dead_code)]
impl Core {
    /// Reads a data word; internal SRAM directly, anything else through [`Core::dm_read`].
    #[inline(always)]
    fn aot_rd(&mut self, addr: u32) -> Result<u32, u32> {
        if addr.wrapping_sub(SRAM_LO) < SRAM_N {
            let o = addr as usize * 4;
            let b = &self.sram[o..o + 4];
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        } else {
            self.dm_read(addr)
        }
    }

    /// Writes a data word; internal SRAM directly (the decode cache is invalidated only when the
    /// word is inside the extent of cached code), anything else through [`Core::dm_write`].
    #[inline(always)]
    fn aot_wr(&mut self, addr: u32, v: u32) -> Result<(), u32> {
        if addr.wrapping_sub(SRAM_LO) < SRAM_N {
            let o = addr as usize * 4;
            self.sram[o..o + 4].copy_from_slice(&v.to_le_bytes());
            if addr >= self.sram_code_nw_lo && addr <= self.sram_code_nw_hi {
                self.invalidate_sram_cache(addr.wrapping_mul(2), 2);
            }
            Ok(())
        } else {
            self.dm_write(addr, v)
        }
    }

    /// Runs the compute field on each element (conditions already known true). `fast` is the
    /// generated out-of-line kernel of this very op (a [`Pe`] run for both elements, one function
    /// per distinct op so the compile stays small); without it the interpreter runs the op.
    #[inline(always)]
    fn aot_compute_both(
        &mut self,
        cop: ComputeOp,
        pc: u32,
        fast: Option<fn(&mut Core)>,
    ) -> Result<(), Fault> {
        let t = super::aot_prof::aot_prof_on().then(super::aot_prof::tsc);
        let r = if let Some(f) = fast {
            f(self);
            Ok(())
        } else {
            self.aot_compute_slow(cop, pc)
        };
        if let Some(t) = t {
            super::aot_prof::prof_compute(fast.is_some(), super::aot_prof::tsc() - t);
        }
        r
    }

    #[inline(never)]
    fn aot_compute_slow(&mut self, cop: ComputeOp, pc: u32) -> Result<(), Fault> {
        let ok_x = self.exec_compute_op(cop);
        let mut ok_y = true;
        if self.simd() {
            self.swap_pe();
            ok_y = self.exec_compute_op(cop);
            self.swap_pe();
        }
        if ok_x && ok_y {
            Ok(())
        } else {
            Err(Fault::Unimplemented {
                pc,
                parcels: [0; 3],
            })
        }
    }

    /// Both elements of `cop` on [`Pe`] state with the tier `kern` (what a generated `kN` does).
    #[inline(always)]
    pub(super) fn aot_pe_both(&mut self, cop: ComputeOp, kern: Kern) {
        let simd = self.simd();
        let mut pe = Pe::load(self, false).with_kern(kern);
        pe.exec(cop);
        pe.store(self);
        if simd {
            let mut pe = Pe::load(self, true).with_kern(kern);
            pe.exec(cop);
            pe.store(self);
        }
    }

    /// Executes the lowered transfer `m` of the instruction at `pc` (the pc is only for faults).
    #[inline(always)]
    pub(super) fn aot_xfer(
        &mut self,
        m: &MemXfer,
        pc: u32,
        fast: Option<fn(&mut Core)>,
    ) -> Result<(), Fault> {
        let bad = |address| Fault::BadAddress { pc, address };
        let ii = usize::from(m.ii);
        if let Data::Modify { dst, src } = m.data {
            if let Some(cop) = m.compute {
                self.aot_compute_both(cop, pc, fast)?;
            }
            let Addr::PostM(mi) = m.addr else {
                return Err(Fault::Unimplemented {
                    pc,
                    parcels: [0; 3],
                });
            };
            self.i[usize::from(dst)] =
                self.modified_always(usize::from(src), self.m[usize::from(mi)]);
            return Ok(());
        }
        match m.data {
            Data::LoadImm { ureg, value } => {
                self.write_ureg(ureg, value);
                if self.simd() && (ureg < 16 || ureg >> 4 == 5) {
                    self.write_ureg(if ureg < 16 { 0x50 | ureg } else { ureg & 15 }, value);
                } else if self.simd() && (ureg == 0x76 || ureg == 0x78) {
                    self.write_ureg(ureg + 1, value);
                }
                return Ok(());
            }
            Data::Swap { r, s } => {
                if let Some(cop) = m.compute {
                    self.aot_compute_both(cop, pc, fast)?;
                }
                let (r, s) = (usize::from(r), usize::from(s));
                self.swap_regs(r, s);
                return Ok(());
            }
            Data::Mov { src, dst } => {
                let cap = self.capture_move(src);
                if let Some(cop) = m.compute {
                    self.aot_compute_both(cop, pc, fast)?;
                }
                self.apply_move_if(dst, src, &cap, true, true);
                return Ok(());
            }
            Data::Shift { op, n, x, ry } => {
                let ok_x = self.shifter_op(op, usize::from(n), self.r[usize::from(x)], ry);
                let mut ok_y = true;
                if self.simd() {
                    self.swap_pe();
                    ok_y = self.shifter_op(op, usize::from(n), self.r[usize::from(x)], ry);
                    self.swap_pe();
                }
                return if ok_x && ok_y {
                    Ok(())
                } else {
                    Err(Fault::Unimplemented {
                        pc,
                        parcels: [0; 3],
                    })
                };
            }
            Data::Compute => {
                if let Some(cop) = m.compute {
                    return self.aot_compute_both(cop, pc, fast);
                }
                return Ok(());
            }
            _ => {}
        }
        let (addr, update) = match m.addr {
            Addr::None => return Ok(()),
            Addr::Direct(a) => (a, None),
            Addr::PostM(mi) => (
                self.dag_out(ii, self.i[ii]),
                Some(self.modified(ii, usize::from(mi))),
            ),
            Addr::PreM(mi) => (
                self.dag_out(ii, self.i[ii].wrapping_add(self.m[usize::from(mi)])),
                None,
            ),
            Addr::PostImm(d) => (self.dag_out(ii, self.i[ii]), Some(self.modified_by(ii, d))),
            Addr::PreImm(d) => (self.dag_out(ii, self.i[ii].wrapping_add(d)), None),
        };
        match m.data {
            Data::Imm(v) => {
                self.aot_wr(addr, v).map_err(bad)?;
                if self.simd() {
                    self.aot_wr(addr.wrapping_add(1), v).map_err(bad)?;
                }
            }
            Data::Reg { ureg, write } => {
                let captured = if write { self.capture(ureg) } else { (0, 0) };
                if let Some(cop) = m.compute {
                    self.aot_compute_both(cop, pc, fast)?;
                }
                let simd = self.simd();
                if write {
                    self.aot_wr(addr, captured.0).map_err(bad)?;
                    if simd {
                        self.aot_wr(addr.wrapping_add(1), captured.1).map_err(bad)?;
                    }
                } else {
                    let v = self.aot_rd(addr).map_err(bad)?;
                    self.write_ureg(ureg, v);
                    let data = ureg < 16 || (0x50..0x60).contains(&ureg);
                    if simd && data {
                        let twin = if ureg < 16 { 0x50 | ureg } else { ureg & 15 };
                        let v = self.aot_rd(addr.wrapping_add(1)).map_err(bad)?;
                        self.write_ureg(twin, v);
                    }
                }
                if !matches!(m.addr, Addr::Direct(_)) {
                    self.broadcast_load(ii, ureg, write);
                }
            }
            _ => {}
        }
        if let Some(u) = update {
            self.i[ii] = u;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::aot_pe::pe_supports;
    use super::super::support::core_at_zero;
    use super::*;

    thread_local! {
        static OP: std::cell::Cell<Option<ComputeOp>> = const { std::cell::Cell::new(None) };
    }

    /// Stand-in for a generated `kN` function.
    fn kernel(core: &mut Core) {
        core.aot_pe_both(OP.with(|o| o.get()).unwrap(), Kern::Exact);
    }

    /// Runs `parcels` on the interpreter and through `aot_xfer` on identical random states and
    /// compares everything, in scalar and SIMD mode.
    fn same_as_interpreter(parcels: &[u16], what: &str) {
        for simd in [false, true] {
            for seed in 1..24u32 {
                let mut a = core_at_zero(parcels);
                let mut b = core_at_zero(parcels);
                let mut x;
                for c in [&mut a, &mut b] {
                    x = seed.wrapping_mul(0x9E37_79B9) | 1;
                    let mut rnd = || {
                        x ^= x << 13;
                        x ^= x >> 17;
                        x ^= x << 5;
                        x
                    };
                    for k in 0..16 {
                        c.r[k] = rnd();
                        c.s[k] = rnd();
                        c.i[k] = 0x9_1000 + (rnd() & 0xFF);
                        c.m[k] = rnd() & 7;
                        c.l[k] = if rnd() & 3 == 0 { 0x20 } else { 0 };
                        c.b[k] = c.i[k] & !0xF;
                    }
                    for k in 0..0x300 {
                        c.dm_write(0x9_1000 + k, rnd()).unwrap();
                        c.dm_write(0x9_2000 + k, rnd()).unwrap();
                    }
                    if simd {
                        c.mode1 |= 1 << 21;
                    }
                    if seed & 4 != 0 {
                        c.mode1 |= 1 << 23 | 1 << 22; // broadcast loads on I1 / I9
                        c.mode1 |= 1 << 24; // CBUFEN
                    }
                }
                let d = a.fetch(0);
                let m = MemXfer::from_insn(&d.insn, d.w)
                    .or_else(|| MemXfer::from_decoded(&DecodedOp::from_insn(0, &d.insn, d.w), d.w))
                    .unwrap_or_else(|| panic!("{what}: not lowered"));
                assert_eq!(a.step(), Ok(()), "{what}");
                // the generated kernel of the op, when the native `Pe` models it
                let fast = m.compute.filter(|&c| pe_supports(c)).map(|c| {
                    OP.with(|o| o.set(Some(c)));
                    kernel as fn(&mut Core)
                });
                b.aot_xfer(&m, 0, fast).unwrap();
                b.pc = a.pc;
                b.steps = a.steps;
                assert_eq!(a.state_mismatch(&b), None, "{what} simd {simd} seed {seed}");
                for k in 0..0x300 {
                    for base in [0x9_1000u32, 0x9_2000] {
                        assert_eq!(
                            a.dm_read(base + k),
                            b.dm_read(base + k),
                            "{what} word {k:#x}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn xfer_equals_the_interpreter_on_random_states() {
        // R0 = DM(I2, M2) post-modify (type 3c) and the store form
        let p = (0b1001u16 << 12) | (2 << 9) | (2 << 6) | (1 << 4);
        same_as_interpreter(&[p], "T3c load");
        same_as_interpreter(&[p | 0x20], "T3c store");
        // I8 = DM(0, I7) (type 15b) and PM form; R0 = DM(-6, I1)
        same_as_interpreter(&[0x9e08, 0x0c00], "T15b ureg");
        same_as_interpreter(&[0x9208, 0x007a], "T15b offset");
        same_as_interpreter(&[0x9228, 0x0a88], "T15b pm");
        // R1 = DM(I6, 2) post-modify immediate (type 4b) and pre-modify
        same_as_interpreter(&[0x6c7e, 0x10bf], "T4 post");
        same_as_interpreter(&[0x6c3e, 0x10bf], "T4 pre");
        // F0 = FLOAT R0 BY R15, R3 = DM(-28, I5) (type 4a with compute)
        same_as_interpreter(&[0x6a3f, 0x218d, 0xa00f], "T4a compute");
        // DM(I0, M1) = 0x...: type 16a, and MODIFY (type 7b)
        same_as_interpreter(&[0x9040, 0x0000, 0x0000], "T16a imm");
        same_as_interpreter(&[0x043e, 0x583f], "T7b modify");
        same_as_interpreter(&[0x043e, 0xe73f], "T7b dst xor");
        // type 14a direct: DM(0x92196) = R0
        same_as_interpreter(&[0x1100, 0x0009, 0x2196], "T14a store");
        same_as_interpreter(&[0x1000, 0x0009, 0x2196], "T14a load");
        // register forms: immediates (data, M, S), swap with compute, moves, shift, compute
        same_as_interpreter(&[0x0f0e, 0x3eb0, 0xf34d], "T17 R14");
        same_as_interpreter(&[0x0f50, 0x000c, 0x0f7c], "T17 S0");
        same_as_interpreter(&[0x0fa1, 0x0001], "T17b M1");
        same_as_interpreter(&[0x7a3e, 0x0480, 0x1001], "T5a swap + compute");
        same_as_interpreter(&[0x703e, 0x933f], "T5 M6 = R1");
        same_as_interpreter(&[0x753e, 0x93bf], "T5 M7 = S1");
        same_as_interpreter(&[0x023e, 0x0032, 0x1fa0], "T6n R10 = BTGL R0 BY 31");
        same_as_interpreter(&[0x013e, 0x0000, 0x1312], "T2 R3 = R1 + R2");
    }

    #[test]
    fn long_word_and_conditional_forms_are_not_lowered() {
        let c = core_at_zero(&[0x1180, 0x0009, 0x2196]); // DM(0x92196) = R0 (LW)
        let d = c.fetch(0);
        assert!(MemXfer::from_insn(&d.insn, d.w).is_none());
        let c = core_at_zero(&[0x6c7e, 0x10bf]);
        let d = c.fetch(0);
        let m = MemXfer::from_insn(&d.insn, d.w).unwrap();
        assert!(m.to_rust().starts_with("MemXfer { ii: "), "{}", m.to_rust());
        assert!(!m.may_change_guard() || matches!(m.data, Data::Reg { write: true, .. }));
    }
}
