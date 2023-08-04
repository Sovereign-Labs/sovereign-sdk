# An example of a `SOV-MODULE`

It demonstrates the following concepts:

### 1. Module structure:

- `lib.rs` contains `ValueSetter` module definition and `sov_modules_api::Module` trait implementation for `ValueSetter`.
- `genesis.rs` contains the module initialization logic.
- `call.rs` contains methods that change module state in response to `CallMessage`.
- `query.rs` contains functions for querying the module state.

### 2. Functionality:

The `admin` (specified in the `ValueSetter` genesis) can update a single `u32` value by creating `CallMessage::SetValue(new_value)` message. Anyone can query the module state by calling the `valueSetter_queryValue` endpoint.

For implementation details, please check comments in the `genesis.rs, call.rs & query.rs`.