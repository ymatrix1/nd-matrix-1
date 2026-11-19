---
# ns4-wmdv
title: 'SHARC Nivel 2: hoist invariantes de loop e remover call frio de FixedMult'
status: completed
type: task
priority: normal
created_at: 2026-11-16T17:15:59Z
updated_at: 2026-11-16T17:20:14Z
parent: ns4-trkb
blocked_by:
    - ns4-s1w0
---

Hoisted loop-invariant MODE1.CBUFEN and M/L/B register loads into the loop preheader via is_dag_cfg_invariant / preload_dag_cfg.

## Summary of Changes
- crates/nord-sharc/src/core/jit.rs: added HoistedDagCfg, is_dag_cfg_invariant, and preload_dag_cfg; wired hoisted preheader values into emit_inline_sram_dag_transfer and emit_inline_dual_sram_load.
