---
# ns4-a7vs
title: "SHARC Nivel 2: inline de buffer circular (dag_next) e FixedMult direto no JIT C\\ jit_dual_move por causa do buffer circular (L != 0).\n\n## Higher Goal\n\nEmitir dag_next (buffer circular com L != 0 e B) diretamente em IR Cranelift em emit_inline_sram_dag_transfer e emit_inline_dual_sram_load, e chamar fixed_mult diretamente sem re-decodificar ComputeOp.\n\n## Acceptance Criteria\n\n- [ ] Helper \\\emit_dag_next_ir\\ em \\crates/nord-sharc/src/core/jit.rs\\ implementando \\Core::dag_next\\ em Cranelift IR (linear e circular)\\\n- [ ] Uso de \\\emit_dag_next_ir\\ em \\\emit_inline_sram_dag_transfer\\ (post-modify) e \\\emit_inline_dual_sram_load\\ (removendo a restricao \\L == 0\\)\\\n- [ ] Chamada direta \\jit_fixed_mult(core, op, n, x, y)\\ para \\ComputeOp::FixedMult\\ sem re-decodificar \\\raw_c\\\\n- [ ] Testes de equivalencia para buffer circular positivo/negativo (\\m >= 0\\ e \\m < 0\\) e medicao de \\\bench_jit_dsp_loop\\ em \\--release --features jit\\\\n- [ ] Checks passando (fmt, clippy com e sem \\jit\\, cargo test --features jit, check_tests.py)"
status: completed
type: task
priority: normal
created_at: 2026-11-16T15:36:19Z
updated_at: 2026-11-16T15:41:58Z
parent: ns4-trkb
blocked_by:
    - ns4-odz6
---

Implemented branchless Cranelift IR circular DAG post-modify (emit_dag_next_ir) with MODE1.CBUFEN and I7/I15 CB7S/CB15S guard, plus direct jit_fixed_mult trampoline.

## Summary of Changes
- crates/nord-sharc/src/core/dag.rs: made MODE1_CBUFEN pub(super).
- crates/nord-sharc/src/core/jit.rs: added emit_dag_next_ir, wired circular DAG post-modify into emit_inline_sram_dag_transfer and emit_inline_dual_sram_load, added jit_fixed_mult trampoline for ComputeOp::FixedMult, and added jit_circular_dag_pos_and_neg_modify_matches_interp.
