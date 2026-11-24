//! Native ARM926EJ-S runtime for the Nord Stage 4 host firmware (`ns4-lle2`, D200).
//!
//! * [`mem`]: sparse paged memory with per-page hook flags and auto-mapping.
//! * [`cpu`]: ARMv5TE interpreter (ARM and Thumb state, CP15 stub).
//! * [`machine`]: the firmware machine (boot stubs, timer interrupt, DSP ports, idle skipping).
//!
//! No vendor bytes live in this crate: the firmware image is loaded at run time from the
//! user's own OS update (D20, D128).

pub mod cpu;
pub mod machine;
pub mod mem;
