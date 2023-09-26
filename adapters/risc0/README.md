# Risc0 Adapter

This package adapts the [Risc0](https://www.risczero.com/) to work as a zkVM for the Sovereign SDK.

## Limitations

Since recursion is not included in the 0.18 release, this adapter is currently limited - individual "slots" may
be proven, but those proofs cannot be recursively combined to facilitate bridging or ultra-fast sync.

## ⚠️ Warning

Risc0 is currently under active development and has not been audited. This adapter has also not been audited. Do not
deploy in production
