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
You can find a complete tutorial showing how to implement a custom module [here](../examples/simple-nft-module/README.md).
Modules typically live in their own crates (you can find a template [here](./module-implementations/module-template/)) so that they're easily
re-usable. A typical struct definition for a module looks something like this:

```rust
#[derive(ModuleInfo)]
pub struct Bank<C: sov_modules_api::Context> {
    /// The address of the bank module.
    #[address]
    pub(crate) address: C::Address,

    /// A mapping of addresses to tokens in the bank.
    #[state]
    pub(crate) tokens: sov_state::StateMap<C::Address, Token<C>>,
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
and the values themselves live in a special struct called a `WorkingSet`.

This has several consequences. First, it means that modules are always cheap to clone. Second it means that calling `my_module.clone()`
always yields the same result as calling `MyModule::new()`. Finally, it means that every method of the module which reads or
modifies state needs to take a `WorkingSet` as an argument.

### Public Functions: The Module-to-Module Interface

The first interface that modules expose is defined by the public methods from the rollup's `impl`. These methods are
accessible to other modules, but cannot be directly invoked by other users. A good example of this is the `bank.transfer_from` method:

```rust
impl<C: Context> Bank<C> {
    pub fn transfer_from(&self, from: &C::Address, to: &C::Address, coins: Coins, working_set: &mut WorkingSet<C>) {
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
to incorporate a _module_ which implements a secure virtual machine that users can invoke to store and run smart contracts.

### The `Call` Function: The Module-to-User Interface

The second interface exposed by modules is the `call` function from the `Module` trait. The `call` function defines the
interface which is _accessible to users via on-chain transactions_, and it typically takes an enum as its first argument. This argument
tells the `call` function which inner method of the module to invoke. So a typical implementation of `call` looks something like this:

```rust
impl<C: sov_modules_api::Context> sov_modules_api::Module for Bank<C> {
	// Several definitions elided here ...
    fn call(&self, msg: Self::CallMessage, context: &Self::Context, working_set: &mut WorkingSet<C>) {
        match msg {
            CallMessage::CreateToken {
                token_name,
                minter_address,
            } => Ok(self.create_token(token_name, minter_address, context, working_set)?),
            CallMessage::Transfer { to, coins } => { Ok(self.transfer(to, coins, context, working_set)?) },
            CallMessage::Burn { coins } => Ok(self.burn(coins, context, working_set)?),
        }
    }
}
```

### The `RPC` Macro: The Node-to-User Interface

The third interface that modules expose is an rpc implementation. To generate an RPC implementation, simply annotate your `impl` block
with the `#[rpc_gen]` macro from `sov_modules_api::macros`.

```rust
#[rpc_gen(client, server, namespace = "bank")]
impl<C: sov_modules_api::Context> Bank<C> {
    #[rpc_method(name = "balanceOf")]
    pub(crate) fn balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<BalanceResponse> {
        Ok(BalanceResponse {
            amount: self.get_balance_of(user_address, token_address, working_set),
        })
    }
}
```

This will generate a public trait in the bank crate called `BankRpcImpl`, which understands how to serve requests with the following form:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "bank_balanceOf",
  "params": { "user_address": "SOME_ADDRESS", "token_address": "SOME_ADDRESS" }
}
```

For an example of how to instantiate the generated trait as a server bound to a specific port, see the [demo-rollup](../examples/demo-rollup/) package.

**Note that only one impl block per module may be annotated with `rpc_gen`**, but that the block may contain as many `rpc_method` annotations as you want.

For an end-to-end walkthrough showing how to implement an RPC server using the Module System, see [here](./RPC_WALKTHROUGH.md)

## Context and Spec: How to Make Your Module System Portable

In addition to `Module`, there are two traits that are ubiquitous in the modules system - `Context` and `Spec`. To understand these
two traits it's useful to remember that the high-level workflow of a Sovereign SDK rollup consists of two stages.
First, transactions are executed in native code to generate a "witness". Then, the witness is fed to the zk-circuit,
which re-executes the transactions in a (more expensive) zk environment to create a proof. So, pseudocode for the rollup
workflow looks roughly like this:

```rust
// First, execute transactions natively to generate a witness for the zkvm
let native_rollup_instance = my_state_transition::<DefaultContext>::new(config);
let witness = Default::default()
native_rollup_instance.begin_slot(witness);
for batch in batches.cloned() {
	native_rollup_instance.apply_batch(batch);
}
let (_new_state_root, populated_witness) = native_rollup_instance.end_batch();

// Then, re-execute the state transitions in the zkvm using the witness
let proof = MyZkvm::prove(|| {
	let zk_rollup_instance = my_state_transition::<ZkDefaultContext>::new(config);
	zk_rollup_instance.begin_slot(populated_witness);
	for batch in batches {
		zk_rollup_instance.apply(batch);
	}
	let (new_state_root, _) = zk_rollup_instance.end_batch();
	MyZkvm::commit(new_state_root)
})
```

This distinction between native _execution_ and zero-knowledge _re-execution_ is deeply baked into the Module System. We take the
philosophy that your business logic should be identical no matter which "mode" you're using, so we abstract the differences between
the zk and native modes behind a few traits.

### Using traits to Customize Your Behavior for Different Modes

The most important trait we use to enable this abstraction is the `Spec` trait. A (simplified) `Spec` is defined like this:

```rust
pub trait Spec {
    type Storage;
    type PublicKey;
    type Hasher;
    type Signature;
}
```

As you can see, a `Spec` for a rollup specifies the concrete types that will be used for many kinds of cryptographic operations.
That way, you can define your business logic in terms of _abstract_ cryptography, and then instantiate it with cryptography which
is efficient in your particular choice of ZKVM.

In addition to the `Spec` trait, the Module System provides a simple `Context` trait which is defined like this:

```rust
pub trait Context: Spec + Clone + Debug + PartialEq {
    /// Sender of the transaction.
    fn sender(&self) -> &Self::Address;
    /// Constructor for the Context.
    fn new(sender: Self::Address) -> Self;
}
```

Modules are expected to be generic over the `Context` type. If a module is generic over multiple type parameters, then the type bound over `Context` is always on the *first* of those type parameters. The `Context` trait gives them a convenient handle to access all of the cryptographic operations
defined by a `Spec`, while also making it easy for the Module System to pass in authenticated transaction-specific information which
would not otherwise be available to a module. Currently, a `Context` is only required to contain the `sender` (signer) of the transaction,
but this trait might be extended in the future.

Putting it all together, recall that the Bank struct is defined like this.

```rust
pub struct Bank<C: sov_modules_api::Context> {
    /// The address of the bank module.
    pub(crate) address: C::Address,

    /// A mapping of addresses to tokens in the bank.
    pub(crate) tokens: sov_state::StateMap<C::Address, Token<C>>,
}
```

Notice that the generic type `C` is required to implement the `sov_modules_api::Context` trait. Thanks to that generic, the Bank struct can
access the `Address` field from `Spec` - meaning that your bank logic doesn't change if you swap out your underlying address schema.

Similarly, since each of the banks helper functions is automatically generic over a context, it's easy to define logic which
can abstract away the distinctions between `zk` and `native` execution. For example, when a rollup is running in native mode
its `Storage` type will almost certainly be [`ProverStorage`](./sov-state/src/prover_storage.rs), which holds its data in a
Merkle tree backed by RocksDB. But if you're running in zk mode the `Storage` type will instead be [`ZkStorage`](./sov-state/src/zk_storage.rs), which reads
its data from a set of "hints" provided by the prover. Because all of the rollups modules are generic, none of them need to worry
about this distinction.

For more information on `Context` and `Spec`, and to see some example implementations, check out the [`sov_modules_api`](./sov-modules-api/) docs.
