# Risc0 Adapter

This package adapts Risc0 version 0.19 to work as a zkVM for the Sovereign SDK.

## Limitations

While in-VM recursion is included in the Risc0 0.19 release, this adapter doesn't currently implement it. Individual "slots" may be proven, but those proofs cannot be recursively combined to facilitate bridging or ultra-fast sync ("user recursion" is not supported).

## Warning

Risc0 is currently under active development and has not been audited. This adapter has also not been audited. Do not
deploy in production
