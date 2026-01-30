# AGENTS.md

This file provides guidance to AI Agents when working with code in this repository.

## Project Overview

Sovereign SDK is a Rust toolkit for building rollups with real-time soft-confirmations, high performance (30k+ UOPS), and pluggable data availability (Celestia, Bitcoin) and zkVM (Risc0, SP1) adapters.

## Build Commands

```bash
make lint                   # Run all linters (fmt, clippy, zepter, dylint)
make lint-fix               # Auto-fix linting issues
make test                   # Run tests with nextest
make check-features         # Verify all feature combinations compile
make mini-ci                # Full pre-submission checks
make install-dev-tools      # Install all development dependencies
```

Instead of running `make build` use `make lint` or `cargo check --all-features` for faster feedback loops.

### Running Single Tests

```bash
cargo nextest run <test_name>
cargo nextest run -p <package_name> <test_name>
```

- Prefer running tests with `-p` to reduce rebuild times.
- Tests must be ran with `nextest`.

### Environment Variables

- `PROPTEST_CASES=50` - Faster local proptest runs (CI uses more)
- `SKIP_GUEST_BUILD=1` - Skip risc0 guest builds during development
- `SP1_SKIP_PROGRAM_BUILD=1` - Skip SP1 program builds during development
- `SOV_TEST_SKIP_DOCKER=1` - Skip docker-based tests, useful for local development without docker

Always use `SKIP_GUEST_BUILD=1` unless performing specific ZK related changes.

## Architecture

### Four Major Layers

1. **Module System** (`crates/module-system/`) - Framework for building rollups
   - `sov-modules-api`: Core traits (Module, Spec, Context)
   - `sov-state`: State management and storage accessors
   - `sov-modules-macros`: Procedural macros for code generation

2. **Full Node** (`crates/full-node/`) - Node components
   - `sov-sequencer`: Transaction acceptance and soft-confirmation production
   - `sov-db`: RocksDB-backed persistent storage
   - `sov-stf-runner`: State transition function executor
   - `sov-blob-sender`: Blob submission manager for DA layer publishing

3. **Adapters** (`crates/adapters/`) - Pluggable integrations
   - DA layers: Celestia, Mock DA
   - zkVMs: Risc0, SP1, Mock ZkVM

4. **Rollup Interface** (`crates/rollup-interface/`) - Core traits
   - StateTransitionFunction, ZkVerifier, DaSpec traits

Delegate to subagents & CLAUDE.md in those directories for more details.

### Bindings

- `./typescript/`: TypeScript bindings for serialization, transaction building, and RPC client
- `./python/`: Python bindings for simple transaction building & serialization in python

### Pre-built Modules (`crates/module-system/module-implementations/`)

- `sov-bank`: Token creation and transfer
- `sov-accounts`: Account management
- `sov-evm`: EVM execution environment
- `sov-sequencer-registry`: Sequencer registration

These are some of the most important modules, but there are others as well.

## Key Patterns

### Generic Spec Pattern

All modules are generic over `Spec`, abstracting cryptographic operations:
```rust
pub struct MyModule<S: sov_modules_api::Spec> {
    #[id]
    pub id: ModuleId,
    #[state]
    pub my_state: StateValue<S::Address, MyData>,
}
```

### Dual-Mode Execution

- **Native Mode**: Direct execution with `DefaultContext`
- **ZK Mode**: Execution within zkVM with `ZkDefaultContext`
- Code gated with `#[cfg(feature = "native")]` runs only in native mode

### State Access Pattern

Modules access state through `WorkingSet<S>` or `TxState<S>`:
```rust
pub fn operation(&self, param: Type, state: &mut impl StateReader<S>) -> Result<Output>
```

## Code Quality Requirements

Avoid over-engineering solutions. Prioritise clarity and maintainability. Don't prematurely add code or features that aren't needed.

### Non-Determinism

- Always avoid non-deterministic code paths in modules and core logic. Because it will break consensus code and ZK proofs.

### Safe Arithmetic

- Use checked/saturating operations for balances, gas calculations, and memory-bound operations
- No float arithmetic (lint denial)

### Feature Gates

- `native` feature: Code not executed in zk proofs
- Most crates use `default-features = false` for zkVM compatibility

### Linting

- Clippy with custom configuration
- Zepter for feature-gate consistency
- Dylint for Sovereign-specific lints
- Nightly rustfmt (use `rustfmt.nightly.toml`)

## Communication

- Always favour clear and concise over verbose. Add more detail when asked
- Always state if you are guessing or making an assumption
- Always link to relevant files, lines, or documentation when referencing code or answering questions
    - Use style `[filename:line_number]` for inline references
    - Use style `[filename]` for general references

## Toolchain

- Rust 1.88.0
- cargo-nextest for testing
- risc0 and SP1 toolchains for ZK proofs
