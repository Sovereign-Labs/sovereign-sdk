# Modules API

Core traits and types that define the module system interface.

## Core Module Trait

```rust
pub trait Module: Clone {
    type Spec: Spec;                    // Execution environment
    type Config;                        // Genesis configuration  
    type CallMessage: CallMessage;      // Transaction types
    type Event: Debug + BorshSerialize + BorshDeserialize + JsonSchema + Send + PartialEq;
    type Error: Debug + Display + Send + Sync + 'static;
    
    fn genesis(&mut self, header: &BlockHeader, config: &Self::Config, state: &mut impl GenesisState) -> Result<()>;
    fn call(&mut self, message: Self::CallMessage, context: &Context, state: &mut impl TxState) -> Result<(), Self::Error>;
    fn charge_gas(&self, state: &mut impl TxState, gas: Gas) -> Result<()>;  // Optional manual gas charging
}
```

## Key Traits and Modules

### Module Organization (`src/`)
- **`module/`** - Core module trait and related types
- **`containers/`** - State storage containers (StateMap, StateVec, StateValue, VersionedStateValue)
- **`state/`** - State access traits and working set implementations
- **`gas/`** - Gas metering and pricing system
- **`transaction/`** - Transaction types and processing
- **`common/`** - Common utilities (addresses, amounts, errors, module IDs)
- **`hooks.rs`** - Pre/post execution hooks
- **`rest/`** - REST API interfaces (native feature)
- **`cli.rs`** - CLI interfaces (native feature)

### Spec Trait
Configures execution environment:
- **`Da`**: Data availability layer specification
- **`Gas`**: Gas unit type
- **`Address`**: Rollup address format (20-64 bytes, secure encoding)
- **`Storage`**: Authenticated state storage (merkle tree variants)
- **`InnerZkvm`/`OuterZkvm`**: ZK verification systems
- **`CryptoSpec`**: Cryptographic primitives

### Context<S: Spec>
Transaction execution context with:
- **`sender`**: Transaction sender address
- **`sequencer`**: Rollup address of sequencer
- **`sequencer_da_address`**: DA layer sequencer address  
- **`gas_refund_recipient`**: Address for gas refunds
- **`execution_context`**: Execution metadata
- **`sequencer_type`**: Type of sequencer (registered/unregistered)
- **`sequencing_data`**: Optional sequencer-provided data
- **`sender_credentials`**: Original signature credentials

### State Containers (`containers/`)

1. **StateMap<K, V>**: Key-value storage with get/set/remove operations
2. **StateVec<T>**: Ordered list with push/pop/index operations  
3. **StateValue<T>**: Single optional value with get/set operations
4. **VersionedStateValue<T>**: Versioned single value storage

Additional variants:
- **AccessoryStateMap/Vec/Value**: For non-consensus auxiliary state
- **KernelStateMap/Vec/Value**: For kernel-level state access
- **Borrowed/BorrowedMut**: Lifetime-managed state borrowing

## State Access Traits and Types

### GenesisState Trait
For one-time module initialization during rollup genesis.

### TxState Trait 
Transaction-scoped state access:
- Read/write state containers
- Automatic gas metering
- Witness tracking for ZK proving

### WorkingSet Type
Concrete implementation providing:
- Merkle tree-backed storage
- Atomic transaction semantics
- State checkpointing and rollback
- Gas accounting integration

## Transaction System (`transaction/`)

- **Transaction types**: v0 and v1 formats with different authentication schemes
- **Credentials**: Digital signature verification  
- **Rewards**: Transaction fee distribution
- **UnsignedTransaction**: Pre-signature transaction format

## Gas System (`gas/`)

- **Gas trait**: Abstract gas units with arithmetic operations
- **Metering**: Automatic tracking of state operations
- **Pricing**: Configurable gas price computation  
- **Meters**: Slot-level, basic, and unlimited gas tracking

## Hook System

Optional cross-module coordination:
- Module-level hooks for transaction lifecycle
- Slot-level hooks for block processing
- Enables complex multi-module interactions

## CLI and REST APIs (Native Feature)

- **CLI**: Command-line interface generation
- **REST**: Automatic REST API generation with OpenAPI specs
- **RPC**: Rollup node RPC interfaces
