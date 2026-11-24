//! Incremental grouping of the stores to the fake DSP ports into ARM to DSP messages
//! (`arm_emu.group_port_writes`). The state lives with the bus, so a message split across two
//! ticks (two drains) keeps its first words.

use super::boot::PAD;
use super::sysbus::{COMM_COUNT, PORT1_BASE, PORT2_BASE};

/// One ARM to DSP message after grouping the port stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    /// 1 or 2 (see `docs/research/arm-dsp-protocol.md`).
    pub path: u8,
    pub comm: u8,
    pub word1: u32,
    pub word2: u32,
    pub pc: u32,
}

const SLOTS: usize = 2 * COMM_COUNT as usize;

/// Per-port partial groups (path 2 ports carry `(address, [pad], value)`, path 1 ports
/// `(word1, word2)`). Fixed size, no allocation.
#[derive(Clone, Debug, Default)]
pub struct Grouper {
    words: [[(u32, u32); 3]; SLOTS],
    len: [u8; SLOTS],
}

/// `(slot, path, comm, words per message)` of a port address.
fn slot_of(port: u32) -> Option<(usize, u8, u32, usize)> {
    let span = 0x10 * COMM_COUNT;
    if (PORT2_BASE..PORT2_BASE + span).contains(&port) {
        let c = (port - PORT2_BASE) / 0x10;
        let need = if PAD[c as usize] != 0 { 3 } else { 2 };
        Some((c as usize, 2, c, need))
    } else if (PORT1_BASE..PORT1_BASE + span).contains(&port) {
        let c = (port - PORT1_BASE) / 0x10;
        Some((COMM_COUNT as usize + c as usize, 1, c, 2))
    } else {
        None
    }
}

impl Grouper {
    /// Feed one store `(port, value, pc)`; returns the message it completes, if any.
    pub fn push(&mut self, port: u32, value: u32, pc: u32) -> Option<Message> {
        let (slot, path, comm, need) = slot_of(port)?;
        let n = self.len[slot] as usize;
        self.words[slot][n] = (value, pc);
        if n + 1 < need {
            self.len[slot] += 1;
            return None;
        }
        self.len[slot] = 0;
        let w = &self.words[slot];
        Some(Message {
            path,
            comm: comm as u8,
            word1: w[0].0,
            word2: w[need - 1].0,
            pc: w[need - 1].1,
        })
    }

    /// Words of unfinished messages.
    pub fn pending_words(&self) -> usize {
        self.len.iter().map(|n| *n as usize).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_follows_pad_and_path() {
        let mut g = Grouper::default();
        let mut out = Vec::new();
        for (p, v, pc) in [
            (PORT2_BASE + 0x10, 0x2800_0010, 1), // comm 1 (padded): addr, pad, value
            (PORT2_BASE + 0x10, 0, 2),
            (PORT2_BASE + 0x10, 0x3F80_0000, 3),
            (PORT1_BASE + 0x30, 0x92414, 4), // comm 3 path 1: two words
            (PORT1_BASE + 0x30, 0x1C0794, 5),
            (PORT2_BASE, 0x2800_0000, 6), // incomplete
        ] {
            out.extend(g.push(p, v, pc));
        }
        assert_eq!(out.len(), 2);
        assert_eq!(
            (out[0].path, out[0].comm, out[0].word1, out[0].word2),
            (2, 1, 0x2800_0010, 0x3F80_0000)
        );
        assert_eq!(
            (out[1].path, out[1].comm, out[1].word2, out[1].pc),
            (1, 3, 0x1C0794, 5)
        );
        assert_eq!(g.pending_words(), 1);
    }

    #[test]
    fn a_message_split_across_two_batches_keeps_its_first_words() {
        let mut g = Grouper::default();
        assert!(g.push(PORT2_BASE + 0x10, 0x2800_0010, 1).is_none());
        assert!(g.push(PORT2_BASE + 0x10, 0, 2).is_none());
        // (a drain happens here in the machine; the grouper is not reset)
        let m = g.push(PORT2_BASE + 0x10, 0xAA, 3).expect("completes");
        assert_eq!((m.word1, m.word2, m.pc), (0x2800_0010, 0xAA, 3));
        assert_eq!(g.pending_words(), 0);
        // the next message is aligned again
        assert!(g.push(PORT2_BASE + 0x10, 7, 4).is_none());
    }

    #[test]
    fn other_addresses_are_ignored() {
        let mut g = Grouper::default();
        assert!(g.push(0x1234, 1, 0).is_none());
        assert_eq!(g.pending_words(), 0);
    }
}
