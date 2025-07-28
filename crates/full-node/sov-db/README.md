# Sovereign DB

This package provides a high-level interface to a [Schema DB](https://github.com/sovereign-Labs/rockbound) designed specifically for use with the Sovereign SDK.
It exposes two db types: `LedgerDb`, and `StateDb`.

## LedgerDb

As the name implies, the `LedgerDb` is designed to store ledger history. It has tables for slots, batches, transactions, and events.
The `LedgerDb` also implements the `LedgerStateProvider` trait, allowing it to easily serve chain history over RPC.

## StateDb

The StateDb is intended to be used with the Jellyfish Merkle Tree provided by the Module System. If you aren't using the
Module System, chances are that you'll want to implement your own State Database.

StateDb is designed to store Jellyfish Merkle Tree data efficiently. It maintains a flat store mapping `(Key, Version)` tuples
to values, as well as a mapping from JMT `NodeKey`s to JMT `Nodes`.

In the Module System, StateDb is abstracted behind the Storage interface, so you won't interact with it directly.
