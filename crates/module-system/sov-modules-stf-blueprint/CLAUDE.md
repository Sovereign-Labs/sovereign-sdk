# STF Blueprint

Implementation of the StateTransitionFunction trait that orchestrates transaction processing for module-based rollups.

## Core Structure

### StfBlueprint<S, RT>

Generic state transition function implementation:
- **S**: Spec type defining crypto, storage, DA layer
- **RT**: Runtime type containing all modules
- Implements `StateTransitionFunction` from rollup interface

### Module Organization (`src/`)
- **`stf_blueprint.rs`**: Core StfBlueprint struct
- **`sequencer_mode/`**: Transaction processing logic
  - `registered.rs`: Registered sequencer batch processing
  - `unregistered.rs`: Unregistered sequencer handling
  - `common.rs`: Shared utilities
- **`proof_processing.rs`**: ZK proof validation
- **`utils.rs`**: Testing utilities

## Key Methods

### StateTransitionFunction Implementation

1. **`apply_slot()`**: Process all blobs in a DA slot
   - Entry point from rollup interface
   - Orchestrates blob selection and processing
   - Manages slot-level state transitions

2. **Internal Processing Pipeline**:
   - **`select_and_validate_blobs()`**: Filter and validate blobs by type
   - **`apply_batches_in_user_space()`**: Process transaction batches
   - **`apply_tx()`** (exported): Process individual transactions

### Blob Types and Processing

The STF handles three blob types:
1. **Sequencer Batches**: From registered sequencers
2. **Single Transactions**: From unregistered sequencers  
3. **ZK Proofs**: For verification and aggregation

### Sequencer Mode Processing

#### Registered Sequencers (`registered.rs`)
- **`apply_batch()`**: Process complete batches
- **`process_tx_and_reward_prover()`**: Individual transaction processing with incentives
- Batch-level gas tracking and receipts
- Prover reward distribution

#### Unregistered Sequencers (`unregistered.rs`) 
- **`apply_batch()`**: Single transaction processing
- No batch structure, direct transaction execution
- Limited gas allocation per transaction

### Gas Management

Multi-level gas tracking:
- **Slot Gas Meter**: Total gas budget for DA slot
- **Batch Gas Meter**: Gas allocation for batch processing
- **Transaction Gas Meter**: Individual transaction limits
- Automatic refunds and fee distribution

### State Management

- **StateCheckpoint**: Atomic state snapshots
- **Working Set**: Transaction-scoped state access
- **Rollback Support**: Failed transactions revert cleanly
- **Event Collection**: Typed events from modules

### Hook Integration

Leverages module system hooks:
- Block-level hooks for slot processing
- Transaction-level hooks for pre/post processing  
- Cross-module coordination and shared state

## Control Flow Injection

Supports `InjectedControlFlow` for customization:
- `NoOpControlFlow`: Standard processing
- Custom implementations for specialized behavior
- Testing and simulation support

## Error Handling

Transaction processing outcomes:
- **Successful**: Changes committed, receipts generated
- **Reverted**: State rolled back, gas consumed
- **Skipped**: Not processed (insufficient gas, validation failure)