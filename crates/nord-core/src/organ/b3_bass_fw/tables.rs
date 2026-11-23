//! Row kinds and ALU operations of the decoded listings (mirrors `tools/sharc_listing_rust.py`).

/// Row kinds.
#[allow(dead_code)]
pub(in crate::organ) mod kind {
    pub const INS: i32 = 0;
    pub const LOOP: i32 = 1;
    pub const SWAP: i32 = 2;
    pub const STORE: i32 = 3;
    pub const LOAD: i32 = 4;
    pub const MODIFY: i32 = 5;
    pub const MOVE_N: i32 = 6;
    pub const MOVE_NI: i32 = 7;
    pub const MOVE_D: i32 = 8;
    pub const MOVE_DN: i32 = 9;
    pub const MOVE_DI: i32 = 10;
    pub const ALU: i32 = 11;
    pub const MRF_SET: i32 = 12;
    pub const COMP: i32 = 13;
    // compiled-only kinds (compile.rs): an item split into a compute and a commit half
    /// ALU result into the temp slot `row[7]` (flags and destination follow at the commit).
    pub const ALU_T: i32 = 14;
    /// Store address and words into the temp slot `row[7]`.
    pub const STORE_T: i32 = 15;
    /// Load words into the temp slot `row[7]`.
    pub const LOAD_T: i32 = 16;
    pub const COMMIT_ALU: i32 = 17;
    pub const COMMIT_STORE: i32 = 18;
    pub const COMMIT_LOAD: i32 = 19;
    /// A conditional ALU item (`if lt`): runs on the elements whose `AN & !AZ` is set.
    pub const COND_ALU: i32 = 20;
}

/// ALU operations of an `ALU` row.
pub(in crate::organ) mod alu_op {
    pub const PASS: i32 = 0;
    pub const FMUL: i32 = 1;
    pub const FADD: i32 = 2;
    pub const FSUB: i32 = 3;
    pub const IADD: i32 = 4;
    pub const ISUB: i32 = 5;
    pub const FLOATBY: i32 = 6;
    pub const ABS: i32 = 7;
    pub const FEXT_R: i32 = 8;
    pub const FEXT_I: i32 = 9;
    pub const FDEP_R: i32 = 10;
    pub const FDEP_I: i32 = 11;
    pub const ASHIFT: i32 = 12;
    pub const LSHIFT: i32 = 13;
    pub const CLIP: i32 = 14;
    pub const AND: i32 = 15;
    pub const ADDI: i32 = 16;
    pub const MULT: i32 = 17;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_follow_the_decoder_enumeration() {
        // `tools/sharc_listing_rust.py`: INS..ALU = range(12), MRF_SET 12, COMP 13; ALU ops in order.
        assert_eq!(
            (kind::INS, kind::ALU, kind::MRF_SET, kind::COMP),
            (0, 11, 12, 13)
        );
        assert_eq!(
            (alu_op::PASS, alu_op::FDEP_I, alu_op::ASHIFT, alu_op::MULT),
            (0, 11, 12, 17)
        );
    }
}
