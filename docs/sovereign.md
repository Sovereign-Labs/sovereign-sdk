# Sovereign

The Sovereign SDK is a generic framework for building zk-rollups. It is designed to be efficiently provable with the Risc0 virtual machine,
to allow seamless interoperation between mutually distrusting rollups, and to support a variety of state-transition functions.

A sovereign SDK rollup consists of three pluggable pieces, a generic Sovereign Node, a chain-specific state transition function, and a
data-availability/consensus layer. The responsibilities of the components are as follows:

- Sovereign Node: provides p2p networking, mempool, RPC, a database, a generic block builder, etc. Stores rollup state/history.
- State Transition Function: implements chain-specific logic for processing transactions
- DA Layer: Provides consensus and data availability. Stores DA layer headers

In addition the Sovereign SDK provides pre-built modules or Pallets which provide components that are common to nearly all blockchains. Example
modules include a Storage pallet (which implements a Merkle-Patricia Trie), a Decentralized Sequencing Pallet, and (importantly) a bridging
pallet

## Interfaces

A Sovereign rollup applies a state transition function (STF) to a set of data which is made available by some underlying blockchain.
The computational work of applying this STF is delegated to an untrusted prover, and the work of verifying the STF is shared between a
zero-knowledge verifier that enforces some polynomial identities (the Risc0 "guest") and the end-user's wallet, which makes a few
lightweight checks in addition to verifying the zk proof. As such, interfaces often define two related methods - one to be run natively
(i.e. by the prover or sequencer), and one to be run in-circuit or by the end-user.

- `ExtractPotentialBlocks(da_header: Da::Header, ) -> `
- `MustProcess<PotentialBlocklMetadata>(&self, metadata: PotentialBlocklMetadata) -> Result<(), ()>`: decides if a given message is "reasonable", meaning that it needs to be considered by the rollup. `PotentialBlocklMetadata` is a generic struct defined by the application. The motivating example for this function is to check whether a given data-blob was submitted by someone who is registered as a rollup sequencer. This method is called on every _potential_ rollup block, that appears on the DA layer, so it should be very lightweight. It is called on all potential blocks as a batch, before any block is actually applied.
- `PrevalidateBlock()`: check
