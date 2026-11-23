# Demo Rollup

Example implementation of a Sovereign SDK rollup demonstrating different DA layer and storage configurations.

## Structure

- `src/` - Rollup implementations and entry point
  - `main.rs` - CLI entry point with configurable DA and storage options
  - `celestia_nomt_rollup.rs` - Celestia DA with NOMT storage
  - `celestia_rollup.rs` - Celestia DA with JMT storage
  - `mock_nomt_rollup.rs` - Mock DA with NOMT storage
  - `mock_rollup.rs` - Mock DA with JMT storage
  - `external_mock_*.rs` - External mock DA variants
  - `lib.rs` - Exports and namespace constants
  - `zk.rs` - ZK proving configurations
  - `sov-cli/` - Transaction submission tool
- `stf/` - State transition function runtime
  - `src/runtime.rs` - Assembled runtime with modules
- `configs/` - Configuration files for different setups
- `provers/` - ZK prover implementations (RISC-0, SP1)

## Main Entry Point

The `main.rs` provides a CLI with options:
- `--da-layer`: Choose between `celestia`, `mock`, or `external-mock`
- `--storage`: Choose between `jmt` or `nomt`
- `--rollup-config-path`: Path to TOML configuration
- `--genesis-config-dir`: Path to genesis configuration

## Key Components

### CelestiaNomtDemoRollup

Located in `celestia_nomt_rollup.rs`, implements:
- `RollupBlueprint` for both `Native` and `WitnessGeneration` modes
- `FullNodeBlueprint` for full node operations
- `WalletBlueprint` for wallet functionality

Key configuration:
- DA: `CelestiaService` for Celestia integration
- Storage: `NomtStorageManager` with NOMT (New Optimized Merkle Tree)
- Prover: `ParallelProverService` with RISC-0 inner VM and Mock outer VM
- State: `NomtProverStorage` for ZK-friendly state access

### Runtime

The runtime (imported from `demo_stf::runtime::Runtime`) assembles all modules.
See `stf/CLAUDE.md` for detailed module list.

## Transaction Flow

1. **DA Service Creation**: Celestia service connects with configured namespaces
2. **Storage Manager**: Initializes NOMT-based storage
3. **Prover Service**: Sets up parallel proof generation
4. **Endpoints**: Creates RPC and additional APIs (Ethereum compatibility)
5. **Rollup Execution**: Processes DA blocks and generates proofs

## Namespaces

Defined in `lib.rs`:
- `ROLLUP_BATCH_NAMESPACE`: For transaction batches (configurable via `BATCH_NAMESPACE`)
- `ROLLUP_PROOF_NAMESPACE`: For ZK proofs (configurable via `PROOF_NAMESPACE`)