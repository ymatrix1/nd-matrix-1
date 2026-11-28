//! Test fixture of the worker: a stand-in translator that reads the code the way the real one
//! does and a small program with instructions of one and three parcels.
//!
//! OWNER: W1-SVC. With the W0 stub `lower::translate_range` returns no blocks, so an equality
//! test between the live core and the scratch core would pass whatever the scratch core held
//! (design P10). [`probe`] depends on every word it reads: it walks the range through
//! `lower::decode::decode_at` (instruction lengths come from the parcels, the last instruction
//! may reach past `hi`) and emits one block per instruction whose `code_hash` covers that
//! instruction's parcels. It stays valid once the real lowering exists because it uses only the
//! decode path and the block types.
use crate::core::support::{core_with, CODE};
use crate::core::uop::config::{UopConfig, UopError};
use crate::core::uop::lower::decode::{code_hash, decode_at};
use crate::core::uop::types::{Block, Uop, UopKind};
use crate::core::Core;

/// NOP, `R3 = R1 + R2` (three parcels), NOP, the add again, NOP: nine parcels, five instructions
/// starting at offsets 0, 1, 4, 5 and 8.
pub(super) fn program() -> Vec<u16> {
    let add = [0x013e, 0x0000, 0x1312];
    let mut p = vec![0x0001];
    p.extend(add);
    p.push(0x0001);
    p.extend(add);
    p.push(0x0001);
    p
}

/// One block and one `INTERP` uop per instruction that starts in `lo..hi`.
pub(super) fn probe(
    core: &mut Core,
    _cfg: &UopConfig,
    lo: u32,
    hi: u32,
) -> Result<(Vec<Block>, Vec<Uop>), UopError> {
    if lo >= hi {
        return Err(UopError::OutOfRange { lo, hi });
    }
    let (mut blocks, mut uops) = (Vec::new(), Vec::new());
    let mut pc = lo;
    while pc < hi {
        let c = decode_at(core, pc);
        let mut b = Block::empty(pc);
        b.end_pc = c.next_pc;
        b.steps = 1;
        b.first = uops.len() as u32;
        b.len = 1;
        b.cum = Box::new([0]);
        b.isa = c.isa;
        b.code_hash = code_hash(core, pc, c.next_pc);
        uops.push(Uop::new(
            UopKind::INTERP,
            1,
            pc,
            u32::from(c.isa),
            c.next_pc,
        ));
        blocks.push(b);
        pc = c.next_pc;
    }
    Ok((blocks, uops))
}

/// A string that differs when any field of any block or uop differs (`Block` has no `PartialEq`).
pub(super) fn render(blocks: &[Block], uops: &[Uop]) -> String {
    format!("{blocks:?} {uops:?}")
}

#[test]
fn the_probe_makes_one_block_per_instruction_and_follows_the_instruction_lengths() {
    let mut c = core_with(&program());
    let cfg = UopConfig::default();
    let (blocks, uops) = probe(&mut c, &cfg, CODE, CODE + 9).unwrap();
    let starts: Vec<u32> = blocks.iter().map(|b| b.pc - CODE).collect();
    assert_eq!(starts, [0, 1, 4, 5, 8]);
    assert_eq!(uops.len(), 5);
    assert_eq!(blocks[1].end_pc, CODE + 4, "the add is three parcels long");
    // the hash follows the parcels of the instruction, not of its neighbours
    let first = blocks[0].code_hash;
    crate::core::support::load(&mut c, CODE + 8, &[0x0000]);
    let (again, _) = probe(&mut c, &cfg, CODE, CODE + 9).unwrap();
    assert_eq!(again[0].code_hash, first);
    assert_ne!(again[4].code_hash, blocks[4].code_hash);
    assert_eq!(
        probe(&mut c, &cfg, 7, 7).unwrap_err(),
        UopError::OutOfRange { lo: 7, hi: 7 }
    );
    assert_ne!(render(&blocks, &uops), render(&again, &uops));
}
