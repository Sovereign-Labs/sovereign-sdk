# Sovereign SDK

Modular rollup framework for building sovereign blockchain applications with ZK proof support.

## Architecture Overview

The SDK provides a modular system for building rollups that can:
- Process transactions from various DA layers (Celestia, Avail, Bitcoin, Mock)
- Generate ZK proofs of execution (RISC-0, SP1)
- Maintain sovereign state independent of the DA layer

## Key Crate Categories

### Core Infrastructure (`crates/`)
- **`module-system/`** - Core framework for building blockchain modules
  - `sov-modules-api/` - Core traits and interfaces
  - `sov-modules-stf-blueprint/` - State transition function orchestration
  - `sov-state/` - Merkle tree-based state storage
  - `module-implementations/` - Pre-built modules (bank, accounts, EVM, etc.)
- **`rollup-interface/`** - Core rollup interfaces and specifications
- **`adapters/`** - DA layer and ZK VM integrations
  - `celestia/`, `avail/`, `mock-da/` - Data availability adapters
  - `risc0/`, `sp1/`, `mock-zkvm/` - Zero knowledge VM adapters
- **`full-node/`** - Full node infrastructure components

### Example Implementation
- **`examples/demo-rollup/`** - Complete rollup implementation
  - `src/main.rs` - CLI entry point with configurable options
  - Multiple rollup variants (Celestia/Mock × JMT/NOMT storage)
  - `stf/src/runtime.rs` - Assembled runtime with modules

## Transaction Flow

1. **DA Layer**: Transactions submitted as blobs to data availability layer
2. **Blob Selection**: Rollup filters and validates relevant blobs
3. **Authentication**: Transaction signatures verified
4. **Execution**: State transition function processes transactions
   - Pre-dispatch hooks
   - Module dispatch based on transaction type
   - Post-dispatch hooks
5. **State Commitment**: Changes committed to merkle tree
6. **Proof Generation**: Optional ZK proof creation

## Key Design Principles

- **Modularity**: Pluggable DA layers, storage backends, and ZK systems
- **Type Safety**: Strong typing with Rust's type system
- **Determinism**: All operations deterministic for ZK proving
- **Gas Metering**: Automatic tracking of compute and storage costs
- **State Isolation**: Each module's state is namespaced