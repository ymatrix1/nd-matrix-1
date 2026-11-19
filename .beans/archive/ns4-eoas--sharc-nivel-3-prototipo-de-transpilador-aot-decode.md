---
# ns4-eoas
title: "SHARC Nivel 3: prototipo de transpilador AOT DecodedOp -> Rust C\\ e atualizacoes de flags mortos ainda limitam o teto do JIT.\n\n## Higher Goal\n\nCriar um prototipo de transpilador AOT (Nivel 3) em \\crates/nord-sharc/src/core/aot.rs\\ que converte regioes de \\DecodedOp\\ (incluindo loops \\DO ... UNTIL LCE\\) em codigo Rust otimizavel pelo \\\rustc\\/LLVM (com opcao \\\fast_float\\).\n\n## Acceptance Criteria\n\n- [ ] Modulo \\crates/nord-sharc/src/core/aot.rs\\ com \\AotOptions\\ e \\Core::emit_aot_rust(start_pc, max_insns, opts)\\`n- [ ] Lowering AOT para \\Nop\\, \\FixedAlu\\, \\FloatMul\\/\\FloatAlu\\ (\\\fast_float\\ nativo ou helper), \\LoadImm\\, \\RegMove\\, \\DagMem\\ e \\FlowOp::DoUntilCounter\\ (\\\for _ in 0..count\\) / \\Return\\`n- [ ] Testes unitarios validando a geracao de codigo Rust para loops \\DO ... UNTIL\\, \\DagMem\\ e \\\fast_float\\`n- [ ] Checks passando (fmt, clippy com e sem \\jit\\, cargo test --features jit, check_tests.py)"
status: completed
type: task
priority: normal
created_at: 2026-11-16T15:19:33Z
updated_at: 2026-11-16T15:21:29Z
parent: ns4-trkb
blocked_by:
    - ns4-570m
---

## Context

Os Niveis 1 (`DecodeCache` + `DecodedOp`) e 2 (`Cranelift JIT`) aceleraram loops de 44.78 para 595-769 Msteps/s, mas chamadas `extern "C"` e atualizacoes de flags mortos ainda limitam o teto do JIT.

## Higher Goal

Criar um prototipo de transpilador AOT (Nivel 3) em `crates/nord-sharc/src/core/aot.rs` que converte regioes de `DecodedOp` (incluindo loops `DO ... UNTIL LCE`) em codigo Rust otimizavel pelo `rustc`/LLVM (com opcao `fast_float`).

## Acceptance Criteria

- [x] Modulo `crates/nord-sharc/src/core/aot.rs` com `AotOptions` e `Core::emit_aot_rust(start_pc, max_insns, opts)`
- [x] Lowering AOT para `Nop`, `FixedAlu`, `FloatMul`/`FloatAlu` (`fast_float` nativo ou `float_mul_public`), `LoadImm`, `RegMove`, `DagMem` e `FlowOp::DoUntilCounter` (`for _ in 0..count`) / `Return`
- [x] Teste unitario `aot_emits_rust_for_do_loop_with_sram_load_add_and_fast_float`
- [x] Checks passando (`fmt`, `clippy` com e sem `jit`, `cargo test -p nord-sharc --features jit --lib`, `check_tests.py`)

## Summary of Changes

- Criado `crates/nord-sharc/src/core/aot.rs` e exposto `AotOptions` + `Core::emit_aot_rust` em `crates/nord-sharc/src/core/mod.rs`.
