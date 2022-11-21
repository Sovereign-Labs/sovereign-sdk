# Stratum SDK

## Overview

The Stratum SDK is a toolkit for developing zk-rollups. It provides two related sets of functionality: a generalized "full
node" implementation that is generic over an internal state transition function ("STF"), and a set of default modules
that can be used within an STF to provide common functionality. The outer "node" implementation is similar to the `tendermint`
package, except that the Stratum node also treats the consensus algorithm as an app - which allows it to support many
different L1s with minimal changes. The set of modules is conceptually similar to the Cosmos SDK, though there are some
differences in the implementation.

A Sovereign SDK chain defines a logical blockchain which is the combination of three distinct elements:

1. An L1 blockchain - which provides DA and consensus
2. A state transition function (written in Rust), which implements some "business logic" running over the
data provided by the L1
3. A zero-knowledge proof system capable of (1) recursion and (2) running arbitrary Rust code

## Glossary

- Rollup: another name for the State Transition Function.
- Slot: a block in the Data Availability layer. May contain many rollup blocks.

## Apps

Following the Cosmos-SDK, we refer to the business logic of our chains as "apps". A rollup consists of two separate apps:
one which provides the consensus, and one which provides the state transition logic. These apps communicate with the full
node and via a set of predefined (required) interfaces. The required functionality of these apps is detailed in the
[Rollup Interface specification](./interface.md).

## Outer Node

### RPC

TODO!

### P2P Network

TODO!

### Database

## Modules

TODO!

## Utilities

### Vector Commitments

The SDK provides two zk-friendly vector commitments - a simple merkle tree (ideal for commiting to simple, static data like
an array of transactions), and a versioned merkle tree (ideal for storing key-value pairs). Both merkle trees should be
generic over hash function (and even hash length) to allow customization for different zkVMs with different efficiency tradeoffs.
