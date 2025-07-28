## Bench generation crate
This crate contains utilities to easily generate a benchmarks of transactions for Sovereign SDK.

To generate a benchmark, one can use the `bench_generator` binary.

To easily build and start benchmark generation one can build the crate (in release mode to ensure good benchmark generation performance).

```
cargo build -r --bin bench_generator
```

Then one can run the benchmark utilities from `./target/release/bench_generator`. Try passing `--help` to get the list of options available.

The available sets of benchmarks are the following:
- **basic**: this is a standard set of benchmarks that try to cover a rather diverse range of slot distribution using the bank and the value setter modules. In particular, it contains the following benchmarks:
    - `bank_transfers_100_percent_address_creation`: contains bank transfers that systematically create new addresses for each transfer. The account state during the execution should be rather big, the execution is minimal (only bank transfers).
    - `bank_transfers`: contains bank transfers that create new addresses with a 50% probability. This serves as witness for the benchmark above and can be useful to compare the state bloat linked with account creation
    - `bank_messages`: generates call messages with all the available call messages from the `Bank` module. This can be useful to compare the cost of specific call messages to the average cost of all the available call messages.
    - `value_setter_set_value`: generates call messages that only perform the `SetValue` operation from the `ValueSetter` module. This call message being extremely straightforward (only one state access), this can be used to evaluate the fixed cost of pre/post execution.
    - `value_setter_messages`: generates call messages that perform both the `SetValue` and `SetManyValues` from the value setter module. This can be useful to evaluate the cost of high memory consumption and call message size (stemming from the additional data of the `SetManyValues` benchmark), and compare it with the simple `value_setter_set_value` benchmark.
    - `mix_bank_transfers_value_setter`: mixes `Bank` transfers with all the call messages from the `ValueSetter`. This can be useful to compare the computation stemming from bank transfers and operations from another module.
    - `complete_bank_value_setter`: this serves as a witness for all the benchmarks above. It contains a rather uniform distribution of call messages from the `Bank` and `ValueSetter` modules.