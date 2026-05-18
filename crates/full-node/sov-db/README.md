# Sovereign DB

This package provides high-level database interfaces designed specifically for use with the Sovereign SDK.
It exposes `LedgerDb`, NOMT state storage, and flat historical/accessory state storage.

## LedgerDb

As the name implies, the `LedgerDb` is designed to store ledger history. It has tables for slots, batches, transactions, and events.
The `LedgerDb` also implements the `LedgerStateProvider` trait, allowing it to easily serve chain history over RPC.

## State Storage

State storage is intended to be used with the NOMT-backed storage implementation provided by the Module System.
If you aren't using the Module System, chances are that you'll want to implement your own state database.

The database stores the authenticated NOMT state alongside flat `(Key, Version)` historical state used by native queries.

In the Module System, state storage is abstracted behind the `Storage` interface, so you won't interact with it directly.
