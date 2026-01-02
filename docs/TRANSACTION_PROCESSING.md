# Transaction Processing in Sovereign SDK

This document describes the complete transaction processing pipeline as implemented by the STF Blueprint (`sov-modules-stf-blueprint`).

## Architecture Overview

The Sovereign SDK uses a modular architecture where:
- **STF Blueprint** orchestrates transaction processing and provides security guarantees
- **Runtime** assembles modules and handles transaction routing via the `DispatchCall` trait
- **Modules** implement business logic within a secure, metered execution environment
- **Capabilities** abstract key security functions (authentication, gas enforcement, etc.)

## Transaction Lifecycle

### Phase 1: DA Layer to Blob Selection

1. **Blob Retrieval**: STF receives blobs from the Data Availability layer via `apply_slot()`
2. **Blob Filtering**: `select_and_validate_blobs()` filters by namespace and blob type
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

1. **Deserialization**: Raw transaction bytes parsed into structured data
2. **Signature Validation**: `R::Auth::authenticate()` verifies cryptographic signatures
3. **Chain ID Validation**: Ensures transaction is for the correct rollup

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
   - Gas usage, events, transaction outcome
   - Success: `SuccessfulTxContents` 
   - Failure: `RevertedTxContents` with error details

### Phase 9: Economic Settlement

**Critical Security Point**: Economic guarantees are **always** enforced regardless of transaction outcome.

1. **Gas Refund**: `refund_remaining_gas()`
   - Returns unused gas to transaction sender
   - **Always succeeds** (marked infallible)

2. **Prover Rewards**: `reward_prover()`
   - Transfers gas fees to prover incentive module
   - **Always succeeds** if transaction reached execution phase
   - **Key Property**: Provers are paid for computational work performed

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
