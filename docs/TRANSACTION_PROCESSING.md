# Transaction Processing in Sovereign SDK

This document describes the complete transaction processing pipeline as implemented by the STF Blueprint (`sov-modules-stf-blueprint`).

## Architecture Overview

The Sovereign SDK uses a modular architecture where:
- **STF Blueprint** orchestrates transaction processing and provides security guarantees
- **Runtime** assembles modules and handles transaction routing via the `DispatchCall` trait
- **Modules** implement business logic within a secure, metered execution environment
- **Capabilities** abstract key security functions (authentication, gas enforcement, etc.)

## Transaction Types and Entry Points

### Transaction Types
- **FullyBakedTx**: Serialized signed transaction ready for DA layer submission (includes authentication info and optional sequencing metadata)
- **RawTx**: Serialized signed transaction that needs authentication encoding before DA submission
- **Transaction<R,S,C>**: The deserialized transaction with two variants:
  - V0: Single signature transaction
  - V1: Multi-signature transaction

### Entry Methods
Transactions enter the system through three paths:
1. **Registered Sequencers**: Submit batches of transactions via `apply_batch()`
2. **Unregistered Sequencers**: Submit single transactions for emergency registration
3. **Proofs**: Submit state transition proofs for verification and aggregation

## Transaction Lifecycle

### Phase 1: DA Layer to Blob Selection

**Location**: `StfBlueprint::select_and_validate_blobs()`

1. **Blob Retrieval**: STF receives blobs from the Data Availability layer via `apply_slot()`
2. **Blob Filtering**: `select_and_validate_blobs()` filters by namespace and blob type
   - Validates blob format and sender credentials
   - Ensures DoS protection by checking reserved gas
   - Returns `BlobSelectorOutput` containing selected blobs to process
3. **Blob Classification**: Three types processed differently:
   - **Sequencer Batches**: From registered sequencers (batch processing)
   - **Single Transactions**: From unregistered sequencers (individual processing)  
   - **ZK Proofs**: For verification and aggregation

### Phase 2: Pre-execution Security Checks

**Location**: `auth_and_process_tx()` in `registered.rs`

Critical security validations that occur **before** transaction deserialization:

1. **Economic Pre-conditions**:
   - Sequencer bond ≥ `max_tx_check_value` (prevents under-bonded sequencers)
   - Slot gas remaining ≥ `max_tx_check_costs` (prevents resource exhaustion)
   - Gas cost calculations don't overflow (prevents arithmetic attacks)

2. **Gas Meter Initialization**:
   - Creates `pre_exec_gas_meter` with exactly `max_tx_check_costs` gas
   - Immediately charges `process_tx_pre_exec_checks_gas()` 
   - All subsequent operations are metered against this budget

**Security Boundary**: These checks protect against DoS attacks where malicious sequencers submit transactions that are expensive to reject.

### Phase 3: Transaction Authentication

**Location**: `deserialize_and_authenticate()` in `registered.rs`

**Critical Security Point**: This is where transaction validity is determined.

**Detailed Authentication Process**:
1. **Hash Calculation**: Compute transaction hash using metered hasher
2. **Deserialization**: Deserialize transaction with gas metering
   - Raw transaction bytes parsed into structured data
   - All operations are gas-metered to prevent DoS
3. **Chain ID Verification**: Ensures transaction targets correct rollup
4. **Signature Verification**: `R::Auth::authenticate()` verifies cryptographic signatures
   - Uses signature cache in native mode for performance optimization
   - Charges gas for verification operations
5. **Returns**: `AuthenticationOutput` containing authenticated transaction data, authorization data, and decoded message

**Error Handling**:
- `AuthenticationError::FatalError`: Sequencer is slashed (invalid signature, wrong chain, etc.)
- `AuthenticationError::OutOfGas`: Sequencer pays authentication costs but isn't slashed
- **Key Security Property**: Invalid transactions **never** result in free work for attackers

### Phase 4: Context Resolution and Authorization

**Location**: `process_tx_and_reward_prover_inner()` in `registered.rs`

1. **Context Resolution**: `transaction_authorizer().resolve_context()` 
   - Maps authentication data to rollup addresses
   - Establishes sender, sequencer, gas payer relationships
   - Can fail with `CannotResolveContext`

2. **Pre-flight Checks**: Custom logic via `InjectedControlFlow`
   - Allows runtime-specific transaction filtering
   - Can result in `RejectedByPreFlight`

### Phase 5: Replay Protection

1. **Uniqueness Check**: `transaction_authorizer().check_uniqueness()`
   - Validates nonce progression (prevents replay attacks)
   - Updates nonce state immediately (marks transaction as attempted)
   - Critical for preventing double-spending

2. **Transaction Marking**: `mark_tx_attempted()`
   - Permanently records transaction attempt
   - Prevents reprocessing even if transaction later fails

**Security Property**: Nonce state is updated **before** gas reservation, ensuring replay protection even for failing transactions.

### Phase 6: Economic Guarantees

1. **Gas Reservation**: `try_reserve_gas()`
   - Locks gas payment from sender's balance in the bank module
   - Prevents transactions from executing without payment
   - Can fail with `CannotReserveGas` if insufficient funds

2. **Gas Budget Transition**: 
   - Moves from pre-execution gas meter (sequencer pays)
   - To transaction gas meter (sender pays)
   - Ensures proper cost attribution

### Phase 7: Module Execution

**Location**: `apply_tx()` in `common.rs`

**Security Boundary**: This is where user code (modules) executes within a secure sandbox.

1. **Working Set Creation**: Transaction gets isolated state access
2. **Authorization Check**: `is_unauthorized_system_tx()` prevents unauthorized system calls
3. **Pre-dispatch Hook**: `runtime.pre_dispatch_tx_hook()` runs first
4. **Module Dispatch**: `runtime.dispatch_call()` routes to appropriate module
5. **Post-dispatch Hook**: `runtime.post_dispatch_tx_hook()` runs last

**Module Security Responsibilities**:
- **Determinism**: All operations must be deterministic for ZK proving
- **Gas Awareness**: Expensive operations should manually charge gas via `Module::charge_gas()`
- **State Validation**: Modules must validate all inputs and state transitions
- **Authorization**: Modules must check permissions before state changes

**STF Guarantees to Modules**:
- **Atomic Execution**: State changes committed or reverted as a unit
- **Gas Metering**: All state operations automatically metered
- **Isolation**: Each transaction has isolated state access via `WorkingSet`
- **Authenticated Context**: Sender and authorization data verified before module execution

### Phase 8: Transaction Finalization

1. **State Commitment**: 
   - Success: `working_set.finalize()` commits all state changes
   - Failure: `working_set.revert()` rolls back all state changes

2. **Receipt Generation**: Creates detailed transaction receipt including:
   - Transaction hash and body
   - Events emitted during execution
   - Gas usage and final status
   - Receipt types:
     - `SuccessfulTxContents`: Transaction executed successfully
     - `RevertedTxContents`: Transaction reverted during execution with error details
     - `SkippedTxContents`: Transaction skipped due to pre-execution failure
     - `IgnoredTxContents`: Special case for transactions processed but not included in normal flow

### Phase 9: Economic Settlement

**Critical Security Point**: Economic guarantees are **always** enforced regardless of transaction outcome.

1. **Gas Refund**: `refund_remaining_gas()`
   - Returns unused gas to transaction sender
   - **Always succeeds** (marked infallible)

2. **Prover Rewards**: `reward_prover()`
   - Transfers gas fees to prover incentive module
   - **Always succeeds** if transaction reached execution phase
   - **Key Property**: Provers are paid for computational work performed

## State Management Architecture

### Working Set Pattern

The SDK uses a sophisticated "Working Set" pattern for transaction state management:

- **WorkingSet**: Provides isolated state access during transaction execution
  - Each transaction operates on its own isolated view of state
  - State changes are tracked but not committed until transaction completion
  - Supports both read and write operations with automatic gas metering

- **StateCheckpoint**: Manages rollback capability and state finalization
  - Captures state at specific points for rollback scenarios
  - Enables atomic transaction processing

- **TxScratchpad**: Temporary state modifications that can be committed or reverted
  - Accumulates all state changes from a transaction
  - Can be materialized into permanent state or discarded

### State Materialization

**Location**: `StfBlueprint::materialize_slot()`

At the end of each DA slot:
1. **State Collection**: Collect all state changes from executed transactions
2. **Root Computation**: Compute new merkle tree state root
3. **Witness Generation**: Generate cryptographic witness for ZK proof generation
4. **Change Set Creation**: Create persistent change set for database storage

### Batch Processing

Each batch produces a `BatchReceipt` containing:
- **Batch Hash**: Unique identifier for the batch
- **Individual Transaction Receipts**: Complete record of each transaction
- **Ignored Transaction Receipts**: Transactions processed but not included in normal flow
- **Batch Metadata**: Gas usage, sequencer information, and processing statistics

## Execution Contexts

The system supports three distinct execution contexts:

1. **SequencerWarmUp**: Pre-execution cache warming
   - Used for optimizing subsequent executions
   - Preparation phase before actual transaction processing

2. **Sequencer**: Execution before DA layer inclusion
   - Transactions executed by sequencer before DA submission
   - Used for ordering and validation

3. **Node**: Execution after DA layer inclusion
   - Standard execution path for included transactions
   - Final authoritative execution

## Gas Metering Architecture

Gas is tracked hierarchically at multiple levels:

- **Transaction Level**: Individual transaction gas limits and consumption
- **Batch Level**: Aggregate gas usage for entire batches
- **Slot Level**: Total gas available per DA layer block
- **Separate Limits**: Different limits for preferred vs standard sequencer transactions

This multi-level approach ensures:
- Fair resource allocation across transaction types
- Prevention of resource monopolization
- Censorship resistance guarantees

## Error Handling and Economic Security

### Transaction Outcomes

1. **Successful**: State committed, events emitted, gas consumed
2. **Reverted**: State rolled back, no events, gas still consumed
3. **Skipped**: Pre-execution failure, transaction not attempted
4. **Ignored**: Special case for transactions that should be processed but not included in normal flow

### Economic Incentive Structure

**For Invalid Signatures/Authentication Failures**:
- **Sequencer Penalty**: Pays from bond via `reward_prover_from_sequencer_balance()`
- **Prover Reward**: Receives authentication cost compensation
- **User Impact**: None (transaction never processed)

**For Failed/Reverted Transactions**:
- **Sequencer**: No penalty (honest mistake)
- **User**: Pays gas costs for attempted execution
- **Prover**: Receives full gas fee payment

**Security Properties**:
- Sequencers are economically incentivized to validate transactions before inclusion
- Provers are guaranteed payment for legitimate computational work
- Users are protected from replay attacks and unauthorized transactions
- DoS attacks are economically infeasible due to gas requirements

## Core System Traits

### StateTransitionFunction Trait
The foundational trait that defines rollup behavior:
- `init_chain()`: Genesis initialization with initial state setup
- `apply_slot()`: Process complete DA layer block and all contained transactions

### Runtime Trait  
Defines the module system's transaction routing behavior:
- `dispatch_call()`: Route transaction calls to appropriate modules based on call type
- **Hook System**: 
  - `pre_dispatch_tx_hook()`: Runs before each transaction execution
  - `post_dispatch_tx_hook()`: Runs after each transaction execution
  - `begin_slot_hook()`: Runs at the start of each slot
  - `end_slot_hook()`: Runs at the end of each slot

These hooks enable cross-module coordination and complex multi-module interactions.

## Security Features

The transaction processing pipeline includes multiple security mechanisms:

1. **Signature Caching**: 
   - Cached signature verification in native mode for performance
   - Prevents redundant cryptographic operations
   - Maintains security while optimizing execution

2. **Comprehensive Gas Protection**: 
   - All operations are gas-metered to prevent DoS attacks
   - Multi-level gas tracking prevents resource exhaustion
   - Economic barriers to malicious behavior

3. **State Isolation**: 
   - Transactions execute in completely isolated state contexts
   - No cross-transaction state contamination
   - Atomic commits ensure consistency

4. **Atomic Transaction Processing**: 
   - State changes are atomic per transaction
   - Complete success or complete rollback
   - No partial state modifications

5. **Censorship Resistance**: 
   - Slot gas limits ensure preferred sequencers cannot monopolize blocks
   - Multiple transaction entry paths prevent single points of failure
   - Economic incentives align with network health

## Security Boundaries and Trust Assumptions

### STF Blueprint Responsibilities (Security-Critical)
- Transaction authentication and signature validation
- Replay protection via nonce management
- Gas metering and economic enforcement
- State isolation between transactions
- Atomic transaction execution

### Module Responsibilities (Application Logic)
- Input validation for module-specific data
- Business logic correctness
- Appropriate gas charging for expensive operations
- State consistency within module domain

### Capabilities System
- Abstracts security-critical functions (auth, gas enforcement)
- Enables pluggable implementations while maintaining security guarantees
- Standard implementations in `StandardProvenRollupCapabilities` provide common patterns

## Transaction Flow Diagram

```
Transaction Entry (DA Layer)
       ↓
Blob Selection & Validation
  ├─ Namespace filtering
  ├─ Credential validation  
  └─ DoS protection checks
       ↓
Authentication (Pre-execution Gas Metered)
  ├─ Hash calculation
  ├─ Deserialization
  ├─ Chain ID verification
  └─ Signature verification
       ↓
Context Resolution & Authorization
  ├─ Address mapping
  ├─ Pre-flight checks
  └─ Relationship establishment
       ↓
Replay Protection
  ├─ Uniqueness check
  └─ Transaction marking
       ↓
Economic Guarantees  
  ├─ Gas reservation
  └─ Gas meter transition
       ↓
Module Execution (Working Set)
  ├─ Pre-dispatch hooks
  ├─ Module dispatch
  └─ Post-dispatch hooks
       ↓
Transaction Finalization
  ├─ State commitment/revert
  └─ Receipt generation
       ↓
Economic Settlement
  ├─ Gas refunds
  └─ Prover rewards
       ↓
State Materialization (End of Slot)
  ├─ State collection
  ├─ Root computation
  ├─ Witness generation
  └─ Change set creation
```

## Audit Focus Areas

1. **Authentication Path**: 
   - Signature validation and chain ID checking in authentication capabilities
   - Gas metering during authentication process
   - Signature caching implementation security

2. **Nonce Management**: 
   - Replay protection logic in uniqueness module
   - Transaction marking timing and atomicity
   - Nonce progression validation

3. **Gas Accounting**: 
   - Multi-level gas metering (slot, batch, transaction)
   - Proper metering and refund logic in gas enforcement
   - Pre-execution vs execution gas attribution

4. **State Isolation**: 
   - Working set implementation and transaction boundaries
   - State checkpoint and rollback mechanisms
   - Cross-transaction state contamination prevention

5. **Economic Incentives**: 
   - Sequencer bonding and prover reward mechanisms
   - Authentication failure penalty logic
   - Economic attack vector analysis

6. **Module Boundaries**: 
   - Proper isolation and capability restriction for user modules
   - Hook system security and module interaction limits
   - State namespace isolation

7. **Concurrency and Atomicity**:
   - Transaction isolation guarantees
   - State finalization atomicity
   - Multi-phase commit integrity

The system implements defense-in-depth security: multiple validation layers ensure malicious transactions are caught early, while economic incentives create strong disincentives for attacks. The modular architecture maintains security boundaries while enabling flexible functionality.
