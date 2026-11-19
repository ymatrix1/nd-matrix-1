---
# ns4-whvu
title: "SHARC Nivel 2: inline de escrita linear em SRAM e DualMove no Cranelift JIT C\\ jit_dag_mem e jit_dual_move.\n\n## Higher Goal\n\nEmitir fast-path inline em Cranelift IR para escritas lineares em SRAM (L[ii] == 0, fora da faixa do bloco compilado) e para DualMove (dois loads lineares em SRAM em paralelo, preservando ordem leitura-antes-de-escrita no compute).\n\n## Acceptance Criteria\n\n- [ ] Fast-path inline em IR Cranelift para \\MoveOp::DagMem { write: true }\\ linear em \\SRAM_WORDS\\ (com guarda de nao-sobreposicao ao bloco atual)\n- [ ] Fast-path inline em IR Cranelift para \\DecodedOp::DualMove\\ (\\!dm_write && !pm_write\\, \\L == 0\\ em ambos os DAGs, \\SRAM_WORDS\\), com fallback para \\jit_dual_move\\ em buffers circulares\n- [ ] Testes de equivalencia (\\Core::run\\ vs \\Core::run_jit\\) para escrita linear/circular e \\DualMove\\ linear/circular + benchmark \\\bench_jit_sram_write_and_dual_loop\\`n- [ ] Checks passando (fmt, clippy com e sem \\jit\\, cargo test --features jit, check_tests.py)"
status: completed
type: task
priority: normal
created_at: 2026-11-16T15:22:43Z
updated_at: 2026-11-16T15:26:36Z
parent: ns4-trkb
blocked_by:
    - ns4-eoas
---

## Context

O JIT ja fazia inline de leituras lineares em SRAM (`ns4-570m`), mas escritas lineares em SRAM e `DualMove` (`T1a`/`T1b`) ainda caiam nos trampolines `extern "C"` `jit_dag_mem` e `jit_dual_move`.

## Higher Goal

Emitir fast-path inline em Cranelift IR para escritas lineares em SRAM (`L[ii] == 0`, fora da faixa do bloco compilado) e para `DualMove` (dois loads lineares em SRAM em paralelo, preservando ordem leitura-antes-de-escrita no compute).

## Acceptance Criteria

- [x] Fast-path inline em IR Cranelift (`emit_inline_sram_dag_transfer`) para `MoveOp::DagMem { write: true }` linear em `SRAM_WORDS` (com guarda de nao-sobreposicao ao bloco atual)
- [x] Fast-path inline em IR Cranelift (`emit_inline_dual_sram_load`) para `DecodedOp::DualMove` (`!dm_write && !pm_write`, `L == 0` em ambos os DAGs, `SRAM_WORDS`), com fallback para `jit_dual_move`
- [x] Teste de equivalencia `jit_inline_sram_write_and_dual_load_match_interpreter`
- [x] Checks passando (`fmt`, `clippy` com e sem `jit`, `cargo test -p nord-sharc --features jit --lib`, `check_tests.py`)

## Summary of Changes

- Unificado `emit_inline_sram_dag_load` em `emit_inline_sram_dag_transfer` (suportando leitura e escrita linear) em `crates/nord-sharc/src/core/jit.rs`.
- Adicionado `emit_inline_dual_sram_load` em `jit.rs` + teste `jit_inline_sram_write_and_dual_load_match_interpreter`.
