<!-- START doctoc generated TOC please keep comment here to allow auto update -->
<!-- DON'T EDIT THIS SECTION, INSTEAD RE-RUN doctoc TO UPDATE -->

- [Demo State Transition Function](#demo-state-transition-function)
  - [Overview](#overview)
  - [Implementing State Transition _Function_](#implementing-state-transition-_function_)
  - [Implementing Runtime: Pick Your Modules](#implementing-runtime-pick-your-modules)
    - [Implementing Hooks for the Runtime:](#implementing-hooks-for-the-runtime)
    - [Exposing RPC](#exposing-rpc)
  - [Make Full Node Itegrations Simpler with the State Transition Runner:](#make-full-node-itegrations-simpler-with-the-state-transition-runner)
    - [Using State Transition Runner](#using-state-transition-runner)
  - [Wrapping Up](#wrapping-up)

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

# Demo State Transition Function

This package shows how you can combine modules to build a custom state transition function. We provide several module implementations
for you, and if you want additional functionality you can find a tutorial on writing custom modules [here](../simple-nft-module/README.md).

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

As you recall, the Module System is primarily designed to help you implement the [State Transition Function
interface](../../rollup-interface/specs/interfaces/stf.md).

That interface is quite high-level - the only notion
that it surfaces is that of a `blob` of rollup data. In the Module System, we work at a much lower level - with
transactions signed by particular private keys. To fill the gap, there's a system called an `AppTemplate`, which
bridges between the two layers of abstraction.

The reason the `AppTemplate` is called a "template" is that it's generic. It allows you, the developer, to pass in
several parameters that specify its exact behavior. In order, these generics are:

1. `Context`: a per-transaction struct containing the message's sender. This also provides specs for storage access, so we use different `Context`
   implementations for Native and ZK execution. In ZK, we read values non-deterministically from hints and check them against a merkle tree, while in
   native mode we just read values straight from disk.
2. `Runtime`: a collection of modules which make up the rollup's public interface

To implement your state transition function, you simply need to specify values for each of these fields.

In the remainder of this section, we'll walk you through implementing each of the remaining generics.

## Implementing Runtime: Pick Your Modules

The final piece of the puzzle is your app's runtime. A runtime is just a list of modules - really, that's it! To add a new
module to your app, just add an additional field to the runtime.

```rust
#[cfg_attr(
    feature = "native",
    expose_rpc(DefaultContext)
)]
#[derive(Genesis, DispatchCall, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct MyRuntime<C: Context> {
    #[allow(unused)]
    sequencer: sov_sequencer_registry::Sequencer<C>,

    #[allow(unused)]
    bank: sov_bank::Bank<C>,

    #[allow(unused)]
    accounts: sov_accounts::Accounts<C>,
}
```

As you can see in the above snippet, we derive four macros on the runtime. The `Genesis` macro generates
initialization code for each module which will get run at your rollup's genesis. The other three macros
allow your runtime to dispatch transactions and queries, and tell it which serialization scheme to use.
We recommend borsh, since it's both fast and safe for hashing.

### Implementing Hooks for the Runtime:

The next step is to implement `Hooks` for `MyRuntime`. Hooks are abstractions that allow for the injection of custom logic into the transaction processing pipeline.

There are two kind of hooks:

`TxHooks`, which has the following methods:

1. `pre_dispatch_tx_hook`: Invoked immediately before each transaction is processed. This is a good time to apply stateful transaction verification, like checking the nonce.
2. `post_dispatch_tx_hook`: Invoked immediately after each transaction is executed. This is a good place to perform any post-execution operations, like incrementing the nonce.

`ApplyBlobHooks`, which has the following methods:

1. `begin_blob_hook `Invoked at the beginning of the `apply_blob` function, before the blob is deserialized into a group of transactions. This is a good time to ensure that the sequencer is properly bonded.
2. `end_blob_hook` invoked at the end of the `apply_blob` function. This is a good place to reward sequencers.

To use the `AppTemplate`, the runtime needs to provide implementation of these hooks which specifies what needs to happen at each of these four stages.

In this demo, we only rely on two modules which need access to the hooks - `sov-accounts` and `sequencer-registry`.

The `sov-accounts` module implements `TxHooks` because it needs to check and increment the sender nonce for every transaction.
The `sequencer-registry` implements `ApplyBlobHooks` since it is responsible for managing the sequencer bond.

The implementation for `MyRuntime` is straightforward because we can leverage the existing hooks provided by `sov-accounts` and `sequencer-registry` and reuse them in our implementation.

```Rust
impl<C: Context> TxHooks for Runtime<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        self.accounts.pre_dispatch_tx_hook(tx, working_set)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        self.accounts.post_dispatch_tx_hook(tx, working_set)
    }
}
```

```Rust
impl<C: Context> ApplyBlobHooks for Runtime<C> {
    type Context = C;

    fn lock_sequencer_bond(
        &self,
        sequencer: &[u8],
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        self.sequencer.lock_sequencer_bond(sequencer, working_set)
    }

    fn reward_sequencer(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        self.sequencer.reward_sequencer(amount, working_set)
    }
}
```

That's it - with those three structs implemented, you can plug them into your `AppTemplate` and get a
complete State Transition Function!

### Exposing RPC

Your modules implement rpc methods via the `rpc_gen` macro, in order to enable the full-node to expose them, annotate the `Runtime` with `expose_rpc`.
In the example above, you can see how to use the `expose_rpc` macro on the `native` `Runtime`.

## Make Full Node Integrations Simpler with the State Transition Runner:

Now that we have an app, we want to be able to run it. For any custom state transition, your full node implementation is going to need a little
customization. At the very least, you'll have to modify our `demo-rollup` example code
to import your custom STF! But, when you're building an STF it's useful to stick as closely as possible to some standard interfaces.
That way, you can minimize the changeset for your custom node implementation, which reduces the risk of bugs.

To help you integrate with full node implementations, we provide standard tools for initializing an app (`StateTransitionRunner`). In this section, we'll briefly show how to use them. Again it is not strictly
required - just by implementing STF, you get the capability to integrate with DA layers and ZKVMs. But, using these structures
makes you more compatible with full node implementations out of the box.

### Using State Transition Runner

The State Transition Runner struct contains logic related to initialization and running the rollup. It has just three methods:

1. `new` - which consumes all the dependencies need for running the rollup.
2. `run` - which runs the rollup.
3. `start_rpc_server` - which exposes an RPC server.


## Wrapping Up

Whew, that was a lot of information. To recap, implementing your own state transition function is as simple as plugging  
a Runtime, a Transaction Verifier, and some Transaction Hooks into the pre-built app template. Once you've done that,
you can integrate with any DA layer and ZKVM to create a Sovereign Rollup.
