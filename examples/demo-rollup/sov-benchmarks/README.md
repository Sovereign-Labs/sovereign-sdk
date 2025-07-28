## Sov-Benchmarks

This crate offers benchmark capabilities for the SDK.

## Submodules

### bench_generator
This module contains utilities to generate benchmarks along with a folder containing the generated benchmarks. 

The [makefile](`./Makefile`) contains useful commands to locally generate benchmarks. One can simply run `make generate_benches` to generate benchmarks. The `SIZE` env variable can be used to specify the size of the benchmarks to be generated. One can also use the `SUBFOLDER` env variable to specify the subfolder where the generated benchmarks should be stored, the `NUM_SLOTS` env variable to specify the number of slots to execute, and the `SEED` env variable to specify the seed value used for the randomization.

You may also directly use the binary's CLI. To print the help, run:
```bash
cargo build -r --bin bench_generator
./target/release/bench_generator --help
```

The README of the binaries [here](`./src/bin/README.md`) contains additional information regarding the benchmarks available.

### bench_runner
This module contains the benchmark runner. It is responsible for running the benchmarks and (optionally) outputs influx metrics to csv files. 

The [makefile](`./Makefile`) contains useful commands to locally run benchmarks. One can use the different `make run_benches_*` commands to run benchmarks with a given preset of parameters. It may be useful to have a quick look at the different parameters/configurations available in the [bench_runner](`./src/bench_runner/mod.rs`) by looking at the commands available in the [Makefile](`./Makefile`).

For more custom use of the benchmark runner, one can directly use the binary's CLI. To print the help, run:
```bash
cargo build -r --bin bench_runner
./target/release/bench_runner --help
```

### Notes:
- By default, the benchmark runner will store the generated benchmarks and metrics in a subfolder of `./src/bench_files`.
- One may need to start a local docker metrics container to be able to store metrics in influx. This can be done by running `make start_metrics_container`. The container can be stopped with `make stop_metrics_container`.
- Note that the benchmark runner may be emitting a large quantity of metrics which may overwhelm the default telegraf configuration. If so, you may want to increase the `metric_batch_size` and the `metric_buffer_limit` in the `telegraf.conf` file.
- It can be useful to check the telegraf logs to see if the metrics are being correctly emitted and none are being dropped.
- When collecting zk-metrics for RISC0, one may want to use the default `bump-allocator` in the zkvm-guest for more cycle-count predicability. This means turning off the `heap-embedded-alloc` feature in the `risc0` guest. Using the linked list allocator increases the noise in the cycle count measurements (reallocating memory is an expensive operation in zk).

### helpers
This module contains helper functions to generate benchmarks.

### node
This module contains basic benchmarking commands to test node execution performances. This defines criterion benchmarks with simple transfers to be able to easily assert node's execution speed.