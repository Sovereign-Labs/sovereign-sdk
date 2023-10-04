# Const Rollup Config

In Sovereign, many state transition functions require consensus critical configuration. For example, rollups on Celestia  
need to configure a namespace which they check for data. This consensus critical configuration needs to be available
to packages at compile time, so that it is baked into the binary which is fed to the zkVM. Otherwise, a malicious
prover might be able to overwrite this configuration at runtime and create valid-looking proofs that were run
over the wrong namespace.

This package demonstrates how you can accomplish such configuration. You can see its usage in the [`main` function of demo-rollup](../demo-rollup/src/main.rs).
