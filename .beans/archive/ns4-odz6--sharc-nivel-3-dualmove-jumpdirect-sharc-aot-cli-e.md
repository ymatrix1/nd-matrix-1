---
# ns4-odz6
title: 'SHARC Nivel 3: DualMove, JumpDirect, sharc-aot CLI e benchmark rustc -O'
status: completed
type: task
priority: normal
created_at: 2026-11-16T15:28:47Z
updated_at: 2026-11-16T15:32:33Z
parent: ns4-trkb
blocked_by:
    - ns4-whvu
---

## Context

O prototipo AOT (`ns4-eoas`) ja transpilava loops `DO ... UNTIL`, `FixedAlu`, `fast_float` e `DagMem` para Rust, mas faltava suporte a `DualMove`, `JumpDirect`, binario CLI `sharc-aot` e medicao real compilada com `rustc -O`.

## Higher Goal

Completar o suporte a `DualMove` e `JumpDirect` no transpilador AOT, adicionar o binario `sharc-aot` e medir o codigo AOT compilado com `rustc -O`.

## Acceptance Criteria

- [x] Suporte a `DecodedOp::DualMove` e `FlowOp::JumpDirect` (state machine `match pc`) em `crates/nord-sharc/src/core/aot.rs`
- [x] Binario CLI `sharc-aot` em `crates/nord-sharc/src/bin/sharc_aot.rs` + registro em `Cargo.toml` + testes unitarios
- [x] Benchmark `bench_aot_rustc_o3_throughput` compilando e rodando o codigo AOT gerado com `rustc -O` (LLVM Scalar Evolution colapsou o loop de 600M steps para 2.5 us)
- [x] Checks passando (`fmt`, `clippy` com e sem `jit`, `cargo test --features jit --all-targets`, `check_tests.py`)

## Summary of Changes

- Expandido `crates/nord-sharc/src/core/aot.rs` com `DualMove`, `JumpDirect` (`cond_expr_rust` + `match pc`), teste `aot_emits_dual_move_and_jump_direct_state_machine` e `bench_aot_rustc_o3_throughput`.
- Criado `crates/nord-sharc/src/bin/sharc_aot.rs` e registrado em `crates/nord-sharc/Cargo.toml`.
