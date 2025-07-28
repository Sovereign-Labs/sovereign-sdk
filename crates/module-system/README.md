# Module System

This directory contains an opinionated framework for building rollups with the Sovereign SDK. It aims to provide a
"batteries included" development experience. Using the Module System still allows you to customize key components of your rollup
like its hash function and signature scheme, but it also forces you to rely on some reasonable default values for things like
serialization schemes (Borsh), address formats (bech32), etc.

By developing with the Module System, you get access to a suite of pre-built modules supporting common functions like generating accounts,
minting and transferring tokens, and incentivizing sequencers. You also get access to powerful tools for generating RPC implementations,
and a powerful templating system for implementing complex state transitions.

## Modules: The Basic Building Block

The basic building block of the Module System is a `module`. Modules are structs in Rust, and are _required_ to implement the `Module` trait.
Modules typically live in their own crates (you can find a template [here](./module-implementations/module-template)) so that they're easily
re-usable. A typical struct definition for a module looks something like this:

```rust
#[derive(Clone, ModuleInfo)]
pub struct Bank<S: sov_modules_api::Spec> {
    /// The ID of the bank module.
    #[id]
    pub(crate) id: ModuleId,

    /// A mapping of addresses to tokens in the bank.
    #[state]
    pub(crate) tokens: sov_state::StateMap<S::Address, Token<S>>,
}
```

At first glance, this definition might seem a little bit intimidating because of the generic `C`.
Don't worry, we'll explain that generic in detail later.
For now, just notice that a module is a struct with an address and some `#[state]` fields specifying
what kind of data this module has access to. Under the hood, the `ModuleInfo` derive macro will do some magic to ensure that
any `#[state]` fields get mapped onto unique storage keys so that only this particular module can read or write its state values.

At this stage, it's also very important to note that the state values are external to the module. This struct definition defines the
_shape_ of the values that will be stored, but the values themselves don't live inside the module struct. In other words, a module doesn't
secretly have a reference to some underlying database. Instead a module defines the _logic_ used to access state values,
and the values themselves live in an external struct whiich implements the `StateReader` and/or `StateWriter` traits.

This has several consequences. First, it means that modules are always cheap to clone. Second it means that calling `my_module.clone()`
always yields the same result as calling `MyModule::new()`. Finally, it means that every method of the module which reads or
modifies state needs to take its state as an argument.

### Public Functions: The Module-to-Module Interface

The first interface that modules expose is defined by the public methods from the rollup's `impl`. These methods are
accessible to other modules, but cannot be directly invoked by other users. A good example of this is the `bank.transfer_from` method:

```rust
impl<S: Spec> Bank<S> {
    pub fn transfer_from(&self, from: &S::Address, to: &S::Address, coins: Coins, state: &mut WorkingSet<S>) {
        // Implementation elided...
    }
}
```

This function transfers coins from one address to another _without a signature check_. If it was exposed to users, it would allow
for the theft of funds. But it's very useful for modules to be able to initiate funds transfers without access to users' private keys. (Of course, modules should be careful to get the user's consent before transferring funds. By
using the transfer_from interface, a module is declaring that it has gotten such consent.)

This leads us to a very important point about the Module System. All modules are _trusted_. Unlike smart contracts on Ethereum, modules
cannot be dynamically deployed by users - they're fixed up-front by the rollup developer. That doesn't mean that the Sovereign SDK doesn't
support smart contracts - just that they live one layer higher up the stack. If you want to deploy smart contracts on your rollup, you'll need
to incorporate a _module_ which implements a secure visible machine that users can invoke to store and run smart contracts.

### The `Call` Function: The Module-to-User Interface

The second interface exposed by modules is the `call` function from the `Module` trait. The `call` function defines the
interface which is _accessible to users via on-chain transactions_, and it typically takes an enum as its first argument. This argument
tells the `call` function which inner method of the module to invoke. So a typical implementation of `call` looks something like this:

```rust
impl<S: sov_modules_api::Spec> sov_modules_api::Module for Bank<S> {
	// Several definitions elided here ...
    fn call(&self, msg: Self::CallMessage, context: &Context<Self::Spec>, state: &mut impl TxState<S>) {
        match msg {
            CallMessage::CreateToken {
                token_name,
                mint_to_address,
            } => Ok(self.create_token(token_name, mint_to_address, context, state)?),
            CallMessage::Transfer { to, coins } => { Ok(self.transfer(to, coins, context, state)?) },
            CallMessage::Burn { coins } => Ok(self.burn(coins, context, state)?),
        }
    }
}
```

### The `RPC` Macro: The Node-to-User Interface

The third interface that modules expose is an rpc implementation. To generate an RPC implementation, simply annotate your `impl` block
with the `#[rpc_gen]` macro from `sov_modules_api::macros`.

```rust
#[rpc_gen(client, server, namespace = "bank")]
impl<S: sov_modules_api::Spec> Bank<S> {
    #[rpc_method(name = "balanceOf")]
    pub(crate) fn balance_of(
        &self,
        user_address: S::Address,
        token_id: TokenId,
        state: &mut impl TxState<S>,
    ) -> RpcResult<BalanceResponse> {
        Ok(BalanceResponse {
            amount: self.get_balance_of(user_address, token_id, state),
        })
    }
}
```

This will generate a public trait in the bank crate called `BankRpcServer`, which understands how to serve requests with the following form:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "bank_balanceOf",
  "params": { "user_address": "SOME_ADDRESS", "token_id": "SOME_ADDRESS" }
}
```

For an example of how to instantiate the generated trait as a server bound to a specific port, see the [demo-rollup](../../examples/demo-rollup) package.

**Note that only one impl block per module may be annotated with `rpc_gen`**, but that the block may contain as many `rpc_method` annotations as you want.

For an end-to-end walkthrough showing how to implement an RPC server using the Module System, see [here](./RPC_WALKTHROUGH.md)

## Context and Spec: How to Make Your Module System Portable

In addition to `Module`, there are two traits that are ubiquitous in the modules system - `Context` and `Spec`. To understand these
two traits it's useful to remember that the high-level workflow of a Sovereign SDK rollup consists of two stages.
First, transactions are executed in native code to generate a "witness". Then, the witness is fed to the zk-circuit,
which re-executes the transactions in a (more expensive) zk environment to create a proof. So, pseudocode for the rollup
workflow looks roughly like this:

```rust
use sov_modules_api::DefaultContext;
fn main() {
    // First, execute transactions natively to generate a witness for the zkVM
    let native_rollup_instance = my_state_transition::<DefaultContext>::new(config);
    let witness = Default::default();
    native_rollup_instance.begin_slot(witness);
    for batch in batches.cloned() {
        native_rollup_instance.apply_batch(batch);
    }
    let (_new_state_root, populated_witness) = native_rollup_instance.end_batch();

    // Then, re-execute the state transitions in the zkVM using the witness
    let proof = MyZkvm::prove(|| {
        let zk_rollup_instance = my_state_transition::<ZkDefaultContext>::new(config);
        zk_rollup_instance.begin_slot(populated_witness);
        for batch in batches {
            zk_rollup_instance.apply(batch);
        }
        let (new_state_root, _) = zk_rollup_instance.end_batch();
        MyZkvm::commit(new_state_root)
    });
}
```

This distinction between native _execution_ and zero-knowledge _re-execution_ is deeply baked into the Module System. We take the
philosophy that your business logic should be identical no matter which "mode" you're using, so we abstract the differences between
the zk and native modes behind a few traits.

### Using traits to Customize Your Behavior for Different Modes

The most important trait we use to enable this abstraction is the `Spec` trait. A (simplified) `Spec` is defined like this:

```rust
pub trait Spec {
    type Address;
    type Storage;
    type PrivateKey;
    type PublicKey;
    type Hasher;
    type Signature;
    type Witness;
}
```

As you can see, a `Spec` for a rollup specifies the concrete types that will be used for many kinds of cryptographic operations.
That way, you can define your business logic in terms of _abstract_ cryptography, and then instantiate it with cryptography, which
is efficient in your particular choice of zkVM.

In addition to the `Spec` trait, the Module System provides a simple `Context` trait which is defined like this:

```rust
pub trait Context: Spec + Clone + Debug + PartialEq {
    /// Sender of the transaction.
    fn sender(&self) -> &Self::Address;
    /// Constructor for the Context.
    fn new(sender: Self::Address) -> Self;
}
```

Modules are expected to be generic over the `Context` type. If a module is generic over multiple type parameters, then the type bound over `Context` is always on the _first_ of those type parameters. The `Context` trait gives them a convenient handle to access all of the cryptographic operations
defined by a `Spec`, while also making it easy for the Module System to pass in authenticated transaction-specific information which
would not otherwise be available to a module. Currently, a `Context` is only required to contain the `sender` (signer) of the transaction,
but this trait might be extended in the future.

Putting it all together, recall that the Bank struct is defined like this.

```rust
pub struct Bank<S: sov_modules_api::Spec> {
    /// The id of the bank module.
    pub(crate) id: ModuleId,

    /// A mapping of addresses to tokens in the bank.
    pub(crate) tokens: sov_state::StateMap<S::Address, Token<S>>,
}
```

Notice that the generic type `C` is required to implement the `sov_modules_api::Context` trait. Thanks to that generic, the Bank struct can
access the `Address` field from `Spec` - meaning that your bank logic doesn't change if you swap out your underlying address schema.

Similarly, since each of the banks helper functions is automatically generic over a context, it's easy to define logic which
can abstract away the distinctions between `zk` and `native` execution. For example, when a rollup is running in native mode
its `Storage` type will almost certainly be [`ProverStorage`](./sov-state/src/prover_storage.rs), which holds its data in a
Merkle tree backed by RocksDB. But if you're running in zk mode the `Storage` type will instead be [`ZkStorage`](./sov-state/src/zk_storage.rs), which reads
its data from a set of "hints" provided by the prover. Because all the rollups modules are generic, none of them need to worry
about this distinction.

For more information on `Context` and `Spec`, and to see some example implementations, check out the [`sov_modules_api`](./sov-modules-api/README.md) docs.

### Module CallMessage and `schemars::JsonSchema`.

Like in the `bank` module the `CallMessage` can be parameterized by `S::Context`. To ensure a smooth wallet experience, we need the `CallMessage` to implement `schemars::JsonSchema` trait. However, simply adding `derive(schemars::JsonSchema)` to the `CallMessage` definition results in the following error:

```
the trait JsonSchema is not implemented for C
```

The reason for this issue is that the standard derive mechanism for `JsonSchema` cannot determine the correct trait bounds for the `Context`. To resolve this, we need to provide the following hint:

```rust
schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "CallMessage")
```

Now, the `schemars::derive` understands that it is sufficient for only `S::Address` to implement `schemars::JsonSchema`

If `CallMessage` in your module uses an associated type from `Context` you might need to provide a similar hint.

### Emitting Typed Events from modules

The module system allows for creating Typed Events and emitting them within the module call logic. To create the types of events to be emitted, first declare an enum with all the variants that could be emitted:

```rust
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
#[cfg_attr(feature = "native", derive(serde::Serialize, serde::Deserialize))]
pub enum Event<S: sov_modules_api::Spec> {
    /// Event for Token Creation
    TokenCreated {
        /// The ID of the token that has been created
        token_id: TokenId,
    },
    /// Event for Token Transfer
    TokenTransferred {
        /// The ID of the token that was transferred
        token_id: TokenId,
        /// The quantity of the token that was transferred
        amount: u128,
    },
}
```

In the above example, the Event enum is generic over the Spec to support emitting the address as part of the variants, but this is not necessary and events can be simpler:

```rust
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
#[cfg_attr(feature = "native", derive(serde::Serialize, serde::Deserialize))]
pub enum Event {
    /// New Value event
    NewValue(u32),
}
```

The above event is from the simple `sov-value-setter` module.
Event variants can also be structs which allow for richer expressiveness and type safety.

Once the Event is created, it needs to be set as the associated type `Event` for the `sov_modules_api::Module` trait implementation

```rust
impl<S: sov_modules_api::Spec> sov_modules_api::Module for Bank<S> {
    // Remaining associated types elided
    type Spec = S;
    type Event = Event<S>;

    // Functions elided
}
```

Once set, you can use a value of the type Event inside any call function and emit it as needed.
`emit_event` has the following signature `fn emit_event(&self, state: &mut impl TxState<Self::Spec>, event_key: &str, event: Self::Event)`

Some things to note -

- `emit_event` will not compile if the event is not of the type `Self::Event` which is set when the `sov_modules_api::Module` trait is implemented
- The internal logic of `emit_event` is built to only emit events when in a native context, so this does not impact code that needs to be proven
- `emit_event` is provided as a blanket trait `EventEmitter` which is implemented for any type that also implements `Module` and `ModuleInfo` traits (refer to previous steps)
- `emit_event` also takes an `event_key` parameter of type `&str`. This key is added to a secondary index internally and makes it easier to narrow down the event search.
- The `event_key` is not constrained to be unique (although it can be if the application requires it). If not unique, the RPC supports pagination (as described later)

To emit an event inside a specific call function, we do the following

```rust
// Import the EventEmitter trait
use sov_modules_api::EventEmitter;

impl<S: sov_modules_api::Spec> Bank<S> {
    pub fn transfer(
        &self,
        to: S::Address,
        coins: Coins,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        // Implementation details elided...
        // Use emit event with a specific value of the type Event that was created and bound to the Module implementation
        self.emit_event(
            state,
            Event::TokenTransferred {
                token_id: coins.token_id,
                amount: coins.amount,
            },
        );
        Ok(())
    }
}
```

Querying events from the node is covered [here](./RPC_WALKTHROUGH.md)
