# Module System

Core framework for building modular blockchain applications in Sovereign SDK.

## Architecture

The module system provides:
- **Trait-based Design**: `Module` trait defines the interface
- **State Management**: Automatic state isolation and persistence
- **Transaction Routing**: Type-safe message dispatch
- **Gas Metering**: Automatic tracking of compute and storage costs
- **Event System**: Typed events for indexing

## Key Components

### Core Traits (`sov-modules-api/`)

1. **Module**: Main trait all modules implement
   - `call()` - Process transactions
   - `genesis()` - One-time initialization
   - `charge_gas()` - Manual gas charging (optional)
   - Associated types for Config, Events, Errors

2. **DispatchCall**: Routes transactions to modules
   - Generated via derive macro
   - Type-safe message routing

3. **ModuleInfo**: Provides module metadata  
   - Module ID and prefix
   - State variable registration
   - Generated via derive macro

### State Transition Blueprint (`sov-modules-stf-blueprint/`)

Implements the state transition function:
- Transaction application logic (`apply_tx`)
- Batch processing (`apply_batches_in_user_space`)
- Slot processing (`apply_slot`)
- Hook management (pre/post dispatch)
- Gas accounting and metering
- Receipt generation

### Module Implementations (`module-implementations/`)

Pre-built modules:
- **sov-bank**: Token transfers and balances
- **sov-accounts**: Account management system
- **sov-chain-state**: Blockchain metadata
- **sov-evm**: Ethereum Virtual Machine
- **sov-prover-incentives**: Prover reward system
- **sov-attester-incentives**: Attester rewards
- **sov-operator-incentives**: Operator rewards
- **sov-paymaster**: Gas payment sponsorship
- **sov-sequencer-registry**: Sequencer management
- **sov-uniqueness**: Nonce/replay protection
- **sov-blob-storage**: DA blob storage
- **sov-value-setter**: Simple key-value storage
- **sov-synthetic-load**: Load testing utilities

### Additional Components

- **sov-modules-macros**: Derive macros for modules
- **sov-modules-rollup-blueprint**: Higher-level rollup assembly
- **sov-address**: Address types and crypto
- **sov-capabilities**: Capability-based access control
- **sov-cli**: Command-line interface generation
- **sov-kernels**: Kernel implementations
- **module-schemas**: JSON schemas for modules

## Transaction Processing

1. **Entry**: Transactions arrive with target module specified
2. **Routing**: Runtime's `DispatchCall` routes to correct module
3. **Execution**: Module's `call()` method processes the transaction
4. **State Updates**: Changes applied to module's state variables
5. **Events**: Module emits typed events
6. **Result**: Success or error returned

## State Management

- **StateMap/StateVec**: Key-value and list storage
- **Automatic Namespacing**: Each module's state is isolated
- **Gas Tracking**: All state operations metered
- **Working Set**: Provides transactional semantics

## Development Pattern

```rust
#[derive(Clone, ModuleInfo)]
pub struct MyModule<S: Spec> {
    #[id]
    pub id: ModuleId,
    
    #[state]
    pub data: StateMap<Key, Value>,
}

impl<S: Spec> Module for MyModule<S> {
    type CallMessage = MyCallMessage;
    
    fn call(&mut self, msg: Self::CallMessage, context: &Context<S>, state: &mut impl TxState<S>) -> Result<(), Error> {
        // Process transaction
    }
}
```