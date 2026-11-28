//! Static step budget of a block and the cumulative steps before each uop.
//!
//! OWNER: W1-LOWER. The budget is what the entry guard compares with the event room
//! (`steps + block.steps <= min(limit, irq.due)`), and `cum[i]` is what mid-block re-entry and
//! the exit sweep use to resume at uop `i`. The skeleton sums `Uop::steps`; the package adds the
//! delay-slot grouping and loop-end rules and proves the static budget equals the interpreter
//! step count for every block.
use super::super::types::Uop;

/// `(total steps, steps accounted before each uop)`. A cumulative value past `u16::MAX` saturates
/// (a block never gets near it: `UopConfig::max_block_steps` is 48 by default).
pub fn budget(uops: &[Uop]) -> (u32, Box<[u16]>) {
    let mut total = 0u32;
    let cum = uops
        .iter()
        .map(|u| {
            let before = u16::try_from(total).unwrap_or(u16::MAX);
            total += u32::from(u.steps);
            before
        })
        .collect();
    (total, cum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::types::UopKind;

    #[test]
    fn the_budget_is_the_sum_of_steps_and_cum_is_the_running_total_before_each_uop() {
        let u = |steps| Uop::new(UopKind::NOP, steps, 0, 0, 0);
        let (total, cum) = budget(&[u(1), u(0), u(2), u(3)]);
        assert_eq!(total, 6);
        assert_eq!(&*cum, &[0, 1, 1, 3]);
        let (total, cum) = budget(&[]);
        assert_eq!((total, cum.len()), (0, 0));
    }

    #[test]
    fn a_huge_block_saturates_cum_instead_of_wrapping() {
        let uops = vec![Uop::new(UopKind::NOP, 255, 0, 0, 0); 300];
        let (total, cum) = budget(&uops);
        assert_eq!(total, 255 * 300);
        assert_eq!(cum[299], u16::MAX);
        assert_eq!(cum[1], 255);
    }
}
