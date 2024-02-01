# `sov-evm` module

The sov-evm module provides compatibility with the EVM.

The module `CallMessage` contains `rlp` encoded Ethereum transaction, which is validated & executed immediately after being dispatched from the DA. Once all transactions from the DA slot have been processed, they are grouped into an `Ethereum` block. Users can access information such as receipts, blocks, transactions, and more through standard Ethereum endpoints.
