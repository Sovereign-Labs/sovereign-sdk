# Sovereign DB

This package provides a high-level interface to a [Schema DB](../sov-schema-db/README.md) designed specifically for use with the Sovereign SDK.
It exposes two db types: `LedgerDB`, and `StateDB`.

## LedgerDB

As the name implies, the `LedgerDB` is designed to store ledger history. It has tables for slots, batches, transactions, and events.
The `LedgerDB` also implements the `LedgerRpcProvider` trait, allowing it to easily serve chain history over RPC.

## StateDB

The StateDB is intended to be used with the Jellyfish Merkle Tree provided by the Module System. If you aren't using the
Module System, chances are that you'll want to implement your own State Database.

StateDB is designed to store Jellyfish Merkle Tree data efficiently. It maintains a flat store mapping `(Key, Version)` tuples
to values, as well as a mapping from JMT `NodeKey`s to JMT `Nodes`.

In the Module System, StateDB is abstracted behind the Storage interface, so you won't interact with it directly.
