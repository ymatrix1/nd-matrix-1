//! Text the event-aware emitter adds to a generated block (D1, `events.rs` has the runtime half).
//! With `ev` off every function returns the v1 text, so v1 modules are byte-identical.

/// First statement of a shared transfer handler: when an interrupt-controller event is due at
/// this boundary, run the instruction on the interpreter (which takes the event) instead.
pub(in crate::core) fn ev_head(ev: bool, len: u32) -> String {
    if ev {
        format!(
            "    if core.steps >= core.irq.due {{ return core.aot_ev_step(pc, pc + {len}); }}\n"
        )
    } else {
        String::new()
    }
}

/// The statement for an instruction without a native lowering.
pub(in crate::core) fn ev_step(ev: bool, pc: u32, next: u32) -> String {
    if ev {
        format!("    core.aot_step_ev(0x{pc:x}, 0x{next:x})?;")
    } else {
        format!("    core.aot_step(0x{pc:x})?;")
    }
}

/// `body` (the native statements of a batch of `n` instructions spanning `span`), run natively
/// only when no event is due inside it; otherwise the interpreter runs the `n` steps.
pub(in crate::core) fn ev_batch(body: &str, n: usize, span: (u32, u32)) -> String {
    format!(
        "    if core.steps + {n} > core.irq.due {{ core.aot_ev_span(0x{:x}, {n}, 0x{:x})?; }} else {{\n{body}    }}\n",
        span.0, span.1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_text_is_unchanged_and_v2_adds_the_event_checks() {
        assert_eq!(ev_head(false, 1), "");
        assert!(ev_head(true, 2).contains("aot_ev_step(pc, pc + 2)"));
        assert_eq!(ev_step(false, 0x10, 0x11), "    core.aot_step(0x10)?;");
        assert!(ev_step(true, 0x10, 0x11).contains("aot_step_ev(0x10, 0x11)"));
        let b = ev_batch("    X\n", 3, (0x10, 0x16));
        assert!(b.contains("steps + 3 > core.irq.due") && b.contains("aot_ev_span(0x10, 3, 0x16)"));
        assert!(b.contains("    X\n    }"));
    }
}
