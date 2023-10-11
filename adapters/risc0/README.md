# Risc0 Adapter

This package adapts Risc0 version 0.18 to work as a zkVM for the Sovereign SDK.

## Limitations

Since in-VM recursion is not included in the 0.18 release, this adapter is currently limited. Individual "slots" may
be proven, but those proofs cannot be recursively combined to facilitate bridging or ultra-fast sync ("user recursion" is not supported).

## Warning

Risc0 is currently under active development and has not been audited. This adapter has also not been audited. Do not
deploy in production
