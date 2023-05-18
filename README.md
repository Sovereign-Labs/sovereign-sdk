<div align="center">
  <h1> Sovereign </h1>
</div>

<div align="center">
  <a href="https://github.com/Sovereign-Labs/sovereign/blob/research/LICENSE">
    <img alt="License: Apache-2.0" src="https://img.shields.io/github/license/cosmos/cosmos-sdk.svg" />
  </a>
  <a href="https://codecov.io/gh/Sovereign-Labs/sovereign" > 
      <img alt="Coverage" src="https://codecov.io/gh/Sovereign-Labs/sovereign/branch/main/graph/badge.svg"/> 
  </a>
</div>

## What is Sovereign?

Sovereign is a free and open-source toolkit for building zk-rollups **that is currently under development**. Sovereign consists of three
logical components.

1. The Sovereign SDK
1. The Sovereign Module System
1. The full node implementation

### The Sovereign SDK: the Core Abstractions

At the heart of Sovereign is the [Sovereign SDK](./sdk/specs/overview.md), which defines the _interfaces_ that rollups
must implement. In the Sovereign SDK, we define a zk-rollup as the combination of three components:

1. A [State Transition Function](./sdk/specs/interfaces/stf.md) ("STF") which defines the "business logic" of the rollup
1. A [Data Availability Layer](./sdk/specs/interfaces/da.md) ("DA layer") which determines the set of transactions that are fed
   to the state transition function
1. A Zero Knowledge proving system (aka "Zero Knowledge Virtual Machine" or "ZKVM"), which takes the compiled rollup code and
   produces succinct proofs that the logic has been executed correctly.

One of the primary goals of the Sovereign SDK is to enable a clean separation of concerns between these three components.
Most rollup developers should not need to implement the DA layer interface - they can write their logic using the SDK,
and be compatible with any DA layer - so deploying their rollup on a new chain is as simple as
picking an [adapter](https://github.com/Sovereign-Labs/Jupiter)
to a specific DA layer off the shelf.

Similarly, teams building DA layers shouldn't need to worry about what kinds of state transitions will be built using their chain.
All they need to do is implement the DA layer interface, and they're automatically compatible with all state transition functions.

The code for the Sovereign SDK lives in the [sdk](./sdk/) folder. For a technical description of the SDK, we recommend the overview
[here](./sdk/specs/overview.md). If you want a less technical introduction, see this [blog post](https://mirror.xyz/sovlabs.eth/pZl5kAtNIRQiKAjuFvDOQCmFIamGnf0oul3as_DhqGA).

### The Sovereign Module System: a Tool for Implementing State Transition Functions

While the Sovereign SDK defines a powerful set of abstractions, it's unopinionated about how a State Transition Function should actually
work. As far as the SDK is concerned, your state machine might have nothing to do with classic "blockchain" financial applications - so
it has no built in notion of "state", accounts, tokens, and the like. This means that the SDK on its own can't offer a
"batteries included" development experience. But one of our goals at Sovereign is to make developing
a rollup as easy as deploying a smart contract. So, we've built out an additional set of tools for defining your state transition function
called the Sovereign Module System.

At the heart of the module system is the package [`sov-modules-api`](./module-system/sov-modules-api/). This package defines
a group of core traits which express how functionality implemented in separate modules can be combined into a `Runtime`
capable of processing transactions and serving RPC requests. It also defines macros for implementing most of these traits.
For many applications, defining your state transition function using the module system should be as simple as picking
some modules off the shelf and defining a struct which glues them together.
To deliver this experience, the module system relies on a set of common types and traits that are used in every module. The
`sov-modules-api` crate defines these traits (like `Context` and `MerkleTreeSpec`) and types like `Address`.

On top of the module API, we provide a [state storage layer](./module-system/sov-state/) backed by a [Jellyfish Merkle Tree](https://github.com/penumbra-zone/jmt)
and a bunch of helpful utilities for working with stateful transactions. Finally, we provide a set of modules implementing common
blockchain functionality like `Accounts`, and fungible `Tokens`.

For more information on the sovereign module system, see its [README](./module-system/README.md). You can also find a tutorial on
implementing and deploying a custom module here (TODO: insert link!)

### The Sovereign Node: a Full Node that "Just Works"

The final component of this repository is the node implementation. This full-node implementation provides an easy way to deploy
and run your rollup. With the default configuration, it can automatically store chain data in its database,
serve RPC requests for chain data and application state, and interact with the DA layer to sync its state and send transactions.
While the full node implementation should be compatible with custom state transition functions, it is currently only tested for
rollups built with the module system. If you encounter any difficulties running the full node, please reach out or open an
issue! All of the core developers can be reached via [Discord](https://discord.gg/kbykCcPrcA).

## Getting Started

### Building a Rollup

The easiest way to build a rollup is to use the Sovereign Module System. You can find a tutorial [here] (TODO: Insert link!).

If you want even more control over your rollup's functionality, you can implement a completely custom State Transition Function
without using the module system. You can find a tutorial [here] (TODO: Insert link!).

### Adding a new Data Availability Layer

If you want to add support for a new data availability layer, the easiest way to get started is to use the
[DA layer adapter template](https://github.com/Sovereign-Labs/da-adapter-template)

## Warning

The Sovereign SDK is Alpha software. It has not been audited and should not be used in production under any circumstances.
API stability and compliance with semantic versioning will be maintained on a best-effort basis.

## License

Licensed under the [Apache License, Version
2.0](./LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
