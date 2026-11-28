//! The verification stack of the uop engine (design section 6): the comparator, snapshots,
//! generators, mutation wrappers, differential fuzz, lockstep, census replay, shadow checking,
//! translation validation and the exit sweep. The interpreter is the oracle for all of them.
//!
//! OWNER: W1-VERIFY (`snap`, `compare`, `gen`, `mutate`) and W3-FUZZ (`fuzz`, `lockstep`,
//! `census`, `shadow`, `tv`, `sweep`). This file only declares the tree. `count_alloc` is the
//! single counting global allocator of the test binary (W0): packages call `allocs_in` and never
//! define their own.
pub mod census;
pub mod compare;
#[cfg(test)]
pub(crate) mod count_alloc;
pub mod fuzz;
pub mod gen;
pub mod lockstep;
pub mod mutate;
pub mod shadow;
pub mod snap;
pub mod sweep;
pub mod tv;

#[cfg(test)]
mod tests {
    use super::compare::FIELDS;
    use super::mutate::Mutant;

    #[test]
    fn field_names_are_unique_and_every_mutant_class_names_one_of_them() {
        let mut names = FIELDS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FIELDS.len());
        assert!(Mutant::ALL.iter().all(|m| FIELDS.contains(&m.field())));
    }
}
