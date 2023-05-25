# Demo State Transition Function

This package shows how you can combine modules to build a custom state transition function. We provide several module implementations
for you, and if you want additional functionality you can find a tutorial on writing custom modules [here](../demo-nft-module/README.md).

For purposes of this tutorial, the exact choices of modules don't matter at all - the steps to combine modules are identical
no matter which ones you pick.

## Overview

To get a fully functional rollup, we recommend implementing two traits. First, there's the [State Transition Function
interface](../../rollup-interface/specs/interfaces/stf.md) ("STF") , which specifies your rollup's abstract logic. Second, there's
a related trait called `State Transition Runner` ("STR") which tells a full node how to instantiate your abstract STF on a concrete machine.

Strictly speaking, it's sufficient for a rollup to only implement the first interface. If you've done that, it's possible to integrate
with ZKVMs and DA Layers - but you'll have to customize your full node implementation a bit to deal with your particular rollup's
configuration. By implementing the STR trait, we make it much easier for the full-node implementation to understand how to interact
with the rollup generically - so we can keep our modifications to the node as minimal as possible. In this demo, we'll implement both traits.

## Implementing State Transition _Function_

As you recall, the module system is primarily designed to help you implement the [State Transition Function
interface](../../rollup-interface/specs/interfaces/stf.md).

That interface is quite high-level - the only notion
that it surfaces it that of a `blob` of rollup data. In the module system, we work at a much lower level - with
transactions signed by particular private keys. To bridge the gap, there's a system called an `AppTemplate`, which
translates between the two layers of abstraction.

The reason the `AppTemplate` is called a "template" is that it's generic. It allows you, the developer, to pass in
several parameters which specify its exact behavior. In order, these four generics are

1. `Context`: a per-transaction struct containing the message's sender. This also provides specs for storage access, so we use different `Context`
   implementations for Native and zk execution. In Zk, we read values non-deterministically from hints and check them against a merkle tree, while in
   native mode we just read values straight from disk.
2. `TxVerifier`: verifies the signatures on transaction and deserializes them into messages
3. `Runtime`: a collection of modules which make up the rollup's public interface
4. `TxHooks`: a set of functions which are invoked at various points in the transaction lifecycle

To implement your state transition function, you simply need to specify values for each of these four fields.

So, a typical app definition looks like this:

```rust
pub type MyNativeStf = AppTemplate<DefaultContext, MyTxVerifier<DefaultContext>, MyRuntime<DefaultContext>, MyTxHooks<DefaultContext>>;
pub type MyZkStf = AppTemplate<ZkDefaultContext, MyTxVerifier<ZkDefaultContext>, MyRuntime<ZkDefaultContext>, MyTxHooks<ZkDefaultContext>>;
```

Note that `DefaultContext` and `ZkDefaultContext` are exported by the `sov_modules_api` crate.

In the remainder of this section, we'll walk you through implementing each of the remaining generics

### Implementing a TxVerifier

The `TxVerifier` interface is defined in `sov-app-template`, and has one associated type and one required method:

```rust
/// TxVerifier encapsulates Transaction verification.
pub trait TxVerifier {
    type Transaction;
    /// Runs stateless checks against a single RawTx.
    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Self::Transaction>;
```

The semantics of the TxVerifier are pretty straightforward - it takes a RawTx (a slice of bytes) as an argument, and does
some work to transform it into some output `Transaction` type _without looking at the current rollup state_. This output transaction
type will eventually be fed to the `TxHooks` for _stateful_ verification.

A typical workflow for a `TxVerifier` is to deserializing the message, and check its signature. As you can see by looking
at the implementation in `tx_verifier_impl.rs`, this is exactly what we do.

```rust
impl<C: Context> TxVerifier for DemoAppTxVerifier<C> {
	// ...
    fn verify_tx_stateless(&self, raw_tx: RawTx) -> anyhow::Result<Self::Transaction> {
        let mut data = Cursor::new(&raw_tx.data);
        let tx = Transaction::<C>::deserialize_reader(&mut data)?;

        // We check signature against runtime_msg and nonce.
        let mut hasher = C::Hasher::new();
        hasher.update(&tx.runtime_msg);
        hasher.update(&tx.nonce.to_le_bytes());

        let msg_hash = hasher.finalize();

        tx.signature.verify(&tx.pub_key, msg_hash)?;
        Ok(tx)
    }
}
```

#### Implementing TxHooks

Once a transaction has passed stateless verification, it will get fed into the execution pipeline. In this pipeline there are four places
where you can inject custom "hooks" using you `TxHooks` implementation.

1. At the beginning of the `apply_blob` function, before the blob is deserialized into a group of transactions. This is a good time to
   apply per-batch validation logic like ensuring that the sequencer is properly bonded
2. Immediately before each transaction is dispatched to the runtime. This is a good time to apply stateful transaction verification, like checking
   the nonce.
3. Immediately after each transaction is executed. This is a good place to perform any post-execution operations, like incrementing the nonce.
4. At the end of the `apply_blob` function. This is a good place to reward sequencers,

To use the app template, you need to provide a TxHooks implementation which specifies what needs to happen at each of these four
stages.

Its common for modules which need access to these hooks to export a `Hooks` struct. If you're relying on an unfamiliar module, be sure to check
its documentation to make sure that you know about any hooks that it may rely on. Your `TxHooks` implementation will usually
just be a wrapper which invokes each of these modules hooks. In this demo, we only rely
on two modules which need access to the hooks - `sov-accounts` and `Sequencer`, so our `TxHooks` implementation only has two fields.

```rust
pub struct DemoAppTxHooks<C: Context> {
    accounts_hooks: accounts::hooks::Hooks<C>,
    sequencer_hooks: sequencer::hooks::Hooks<C>,
}
```

You can view the full implementation in `tx_hooks_impl.rs`

### Implementing Runtime: Pick Your Modules

The final piece of the puzzle is your app's runtime. A runtime is just a list of modules - really, that's it! To add a new
module to your app, just add an additional field to the runtime.

```rust
#[derive(Genesis, DispatchCall, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct MyRuntime<C: Context> {
    #[allow(unused)]
    sequencer: sequencer::Sequencer<C>,

    #[allow(unused)]
    bank: bank::Bank<C>,

    #[allow(unused)]
    accounts: sov_accounts::Accounts<C>,
}
```

As you can see in the above snippet, we derive four macros on the runtime. The `Genesis` macro generates
initialization code for each module which will get run at your rollup's genesis. The other three macros
allow your runtime to dispatch transactions and queries, and tell it which serialization scheme to use.
We recommend borsh, since it's both fast and safe for hashing.

That's it - with those three structs implemented, you can them into your AppTemplate and get a complete State Transition Function!

## Make Full Node Itegrations Simpler with the State Transition Runner trait:

Now that we have an app, we want to be able to run it. For any custom state transition, your full node implementation is going to need a little
customization. At the very least, you'll have to modify our `demo-rollup` example code
to import your custom STF! But, when you're building an STF it's useful to stick as closely as possible to some standard interfaces.
That way, you can minimize the changeset for your custom node implementation, which reduces the risk of bugs.

To help you integrate with full node implementations, we provide standard traits for intitializing an app (`StateTransitionRunner`) and
starting an RPC server (`RpcRunner`). In this section, we'll briefly show how to implement both traits. Again, neither trait is stricly
required - just by implementing STF, you get the capability to integrate with DA layers and Zkvms. But, implementing these traits
makes you more compatible with full node implementations out of the box

### Implementing State Transition Runner

The State Transition Runner trait contains logic related to intitialization. It has just three methods:

1. `new` - which allows us to instantiate a state transition function using a `RuntimeConfig` specific to the particular execution mode.
   For example, when you're running a prover you likely want to configure a standard RocksDB instance - but in zk mode, you have to
   set up your STF to read from a merkle tree instead. Using STR, we can easily swap out this configuration.
2. `inner` - which returns an immutable reference to the inner state transition function
3. `inner mut` - which returns a mutable reference to the inner STF

As you can see in the demo codebase, we implement StateTransitionRunner two different times for the DemoAppRunner struct - once for `Prover` mode
and once for `Zk` mode.

The `Prover` implementation is gated behind the `native` feature flag. This flag is what we use in the SDK to mark code which can only be run
outside of the zk-circuit. Since this implementation will always run on a physical machine, we can annotate it with the
`expose_rpc` macro telling it to enable RPC queries against the Bank, Election, and ValueSetter modules.
We'll cover this macro in more detail in the next section.

```rust
pub struct DemoAppRunner<C: Context, Vm: Zkvm>(pub DemoApp<C, Vm>);

#[cfg(feature = "native")]
#[expose_rpc((Bank<DefaultContext>,Election<DefaultContext>,ValueSetter<DefaultContext>))]
impl<Vm: Zkvm> StateTransitionRunner<ProverConfig, Vm> for DemoAppRunner<DefaultContext, Vm> {
    type RuntimeConfig = Config;
    type Inner = DemoApp<DefaultContext, Vm>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let storage = ProverStorage::with_config(runtime_config.storage)
            .expect("Failed to open prover storage");
        let app = AppTemplate::new(storage, Runtime::new(), DemoAppTxVerifier::new(), DemoAppTxHooks::new());
        Self(app)
    }
	// ...
}

impl<Vm: Zkvm> StateTransitionRunner<ZkConfig, Vm> for DemoAppRunner<ZkDefaultContext, Vm> {
    type RuntimeConfig = [u8; 32];
    type Inner = DemoApp<ZkDefaultContext, Vm>;

    fn new(runtime_config: Self::RuntimeConfig) -> Self {
        let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
        let app = AppTemplate::new(storage, Runtime::new(), DemoAppTxVerifier::new(), DemoAppTxHooks::new());
        Self(app)
    }
	// ...
}
```

### Exposing RPC

If any of your modules expose rpc methods via the `rpc_gen` macro, there are two things that you should do in your STF package to
enable the full-node to expose them

1. Annotate the state transition runner with the specific modules you want to expose with `expose_rpc`
2. Implement the `RpcRunner` trait. provide an implementation for the `get_storage` function

You can see an example of how to use the `expose_rpc` macro on the `native` `StateTransitionRunner` implementation. That macro
takes a tuple of modules (with the appropriate generics) as arguments, and generates RPC servers for each one. In order to
make those generated RPC servers work, though, we need to provide them with access to the database. This is where the RpcRunner
trait comes in.

```rust
#[cfg(feature = "native")]
impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
    type Context = DefaultContext;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage {
        self.inner().current_storage.clone()
    }
}
```

## Wrapping Up

Whew, that was a lot of information. To recap, implementing your own state transition function is as simple as plugging  
a Runtime, a Transaction Verifier, and some Transaction Hooks into the pre-built app template. Once you've done that,
you can integrate with any DA layer and Zkvm to create a Sovereign Rollup.

Everything else in this tutorial is about making it easy to _run_ your Sovereign Rollup. To be as compatible as
possible, state transitions usually want to implement the StateTransitionRunner and RpcRunner traits. For more
detail about integrating your STF into a full-node, check out the [`demo-rollup` example](../demo-rollup/).
