# Risc0 Adapter

This package adapts Risc0 version 0.14 to work as a ZKVM for the Sovereign SDK.

## Limitations

Since recursion is not included in the 0.14 release, this adapter is currently limited - individual "slots" may
be proven, but those proofs cannot be recursively combined to facilitate bridging or ultra-fast sync.

## Warning

Risc0 is currently under active development and has not been audited. This adapter has also not been audited. Do not
deploy in production
