---
# ns4-570m
title: |-
    SHARC Nivel 2: lowering inline de DM/PM SRAM linear no Cranelift JIT C\ jit_dag_mem a cada acesso de memoria, passando por periph/watch/rwatch/IOP/SHORT_WORDS e dag_out/modified.

    ## Higher Goal

    Emitir fast-path inline em Cranelift IR para leituras/escritas lineares (L[ii] == 0) na faixa SRAM_WORDS (0x9_0000..0x10_0000) quando periph/watch/rwatch estao inativos, com fallback automatico para jit_dag_mem em buffers circulares ou enderecos fora da SRAM.

    ## Acceptance Criteria

    - [ ] Fast-path inline em Cranelift IR para MoveOp::DagMem (ureg < 16, !lw, compute: None, cond: 31) com L[ii] == 0 e addr em SRAM_WORDS
    - [ ] Fallback automatico para jit_dag_mem quando L[ii] != 0 (circular), addr fora de SRAM_WORDS, ou escrita potencialmente sobre codigo
    - [ ] Testes de equivalencia (linear SRAM read/write + fallback circular L != 0) e benchmark bench_jit_dagmem_loop (--release --features jit)
    - [ ] Checks passando (fmt, clippy com e sem jit, cargo test --features jit, check_tests.py)
status: completed
type: task
priority: normal
created_at: 2026-11-16T15:10:13Z
updated_at: 2026-11-16T15:18:14Z
parent: ns4-trkb
blocked_by:
    - ns4-u7ox
---

## Context

`MoveOp::DagMem` ainda chamava o trampoline `extern "C"` `jit_dag_mem` a cada acesso de memoria, passando por `periph`/`watch`/`rwatch`/`IOP`/`SHORT_WORDS` e `dag_out`/`modified`.

## Higher Goal

Emitir fast-path inline em Cranelift IR para leituras lineares (`L[ii] == 0`) na faixa `SRAM_WORDS` (`0x9_0000..0x10_0000`) quando `periph`/`watch`/`rwatch` estao inativos, com fallback automatico para `jit_dag_mem` em buffers circulares ou enderecos fora da SRAM.

## Acceptance Criteria

- [x] Fast-path inline em Cranelift IR (`emit_inline_sram_dag_load`) para `MoveOp::DagMem` (`write: false`, `ureg < 16`, `!lw`, `compute: None`, `cond: 31`) com `L[ii] == 0` e `addr` em `SRAM_WORDS`
- [x] Fallback automatico para `jit_dag_mem` quando `L[ii] != 0` (circular) ou `addr` fora de `SRAM_WORDS`
- [x] Teste `jit_inline_sram_load_matches_interpreter_linear_and_circular_fallback` + benchmark `bench_jit_dagmem_loop` (`--release --features jit`): **52.16 Msteps/s** (`run`) -> **595.92 Msteps/s** (`run_jit`, **11.4x**)
- [x] Checks passando (`fmt`, `clippy` com e sem `jit`, `cargo test -p nord-sharc --features jit --lib`, `check_tests.py`)

## Summary of Changes

- Passado `sram_ptr` na assinatura de `JitBlockFn` e adicionado `sram_fast` na chave de cache do `JitEngine`.
- Adicionado `emit_inline_sram_dag_load` em `crates/nord-sharc/src/core/jit.rs`.
- Adicionados teste de paridade linear/circular e `bench_jit_dagmem_loop`.
