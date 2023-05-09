### This is a simple example of a `SOV-MODULE`.

It demonstrates the following concepts:
1. Module structure
    - `lib.rs` contains `ValueSetter` module definition and `sov_modules_api::Module` trait implementation for `ValueSetter`
    - `genesis.rs` contains the module initialization logic.
    - `call.rs` contains methods that change module state in response to `CallMessage`
    - `query.rs` contains function for querying the module state.

2. The `ValueSetter` module shows how to interact with state (in our case `u32`). In order 
