# Sov Soak Testing

This package provides two binaries - a bare-bones rollup the provides the `bank` and `paymaster` modules, and a transaction generator which submits txs via HTTP. 


## Getting Started
1. Start the rollup `cargo run --release`
2. Start the generator `cargo run --bin generator`.

Note: Although both packages are zero-config, they provide simple CLIs in case you need to override any default settings. For example, you could run the generator on a different machine as long as you specify the correct REST API address using the CLI. 
