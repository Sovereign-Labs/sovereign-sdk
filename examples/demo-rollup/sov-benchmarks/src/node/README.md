<!-- START doctoc generated TOC please keep comment here to allow auto update -->
<!-- DON'T EDIT THIS SECTION, INSTEAD RE-RUN doctoc TO UPDATE -->
**Table of Contents**  *generated with [DocToc](https://github.com/thlorenz/doctoc)*

- [Native Benchmarks](#native-benchmarks)
  - [Methodology](#methodology)
- [Makefile](#makefile)

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

# Native Benchmarks
Native benchmarks refer to the performance of the rollup SDK in native mode - this does not involve proving
## Methodology
* We use the Bank module's Transfer call as the main transaction for running this benchmark. So what we're measuring is the number of value transfers that can be done per second. 
* We do not connect to the DA layer since that will be the bottleneck if we do. We pre-populate 100 blocks (configurable via env var SOV_BENCH_BLOCKS) with 1 blob each containing 1000 transactions each (configurable via env var SOV_BENCH_TXNS_PER_BLOCKS). 
* The first block contains a "CreateToken" and "Mint" transactions.
* All token transfers are initiated from the created token's mint address

We use two scripts for benchmarking:
* **rollup_bench.rs**: This makes use of the rust criterion benchmarking framework. 
  * One issue with this is that most benching frameworks are focused on micro-benchmarks for pure functions. 
  * To get a true estimate of TPS we need to write to disk and this has a side effect for the bench framework and when it tries executing the same writes.
  * Bench frameworks (criterion, glassbench) take an iterator as an argument, and we cannot control the number of iterations directly. The framework chooses the sampling and the number of iterations.
  * This benchmark prepares rollup state by processing the number of blocks and writing data to the disk. The last block is benchmarked by criterion, but output of it is thrown away, so it is possible to have micro-benchmark vibe in it.
  * The output of the framework is the mean time for processing a single block (containing the configured number of transactions)
```
Going to bench after 100 blocks, with 1000 unique senders.
Each block will have sov_bank::Bank::Transfer call message from each sender to random address.
Meaning that when bench start there will be 100000 transfers in a tree plus minting for each sender in the beginning.
rollup main stf loop    time:   [628.96 ms 633.65 ms 638.84 ms]
                        change: [-1.6769% -0.7757% +0.1188%] (p = 0.11 > 0.05)
                        No change in performance detected.
Found 8 outliers among 100 measurements (8.00%)
  7 (7.00%) high mild
  1 (1.00%) high severe
```
* **rollup_coarse_measure.rs**
  * This script uses coarse grained timers (with std::time) to measure the time taken to process all the pre-generated blocks.
  * We can control the number of blocks and transactions per block with environment variables
  * There are timers around the main loop for a total measurement, as well as timers around key functions
    * begin_slot
    * apply_blob
    * end_slot
  * The script uses rust lib prettytable-rs to format the output in a readable way
  * Optionally, the script also allows generating prometheus metrics (histogram), so they can be aggregated by other tools.
```
+--------------------+--------------------+
| Blocks             | 100                |
+--------------------+--------------------+
| Txns per Block     | 10000              |
+--------------------+--------------------+
| Total              | 292.819598958s     |
+--------------------+--------------------+
| Begin slot         | 39.414µs           |
+--------------------+--------------------+
| End slot           | 243.091403746s     |
+--------------------+--------------------+
| Apply Blob         | 46.639351922s      |
+--------------------+--------------------+
| Txns per sec (TPS) | 3424.6575342465753 |
+--------------------+--------------------+
```

# Makefile
We abstract having to manually run the benchmarks by using a Makefile for the common benchmarks we want to run

The Makefile is located in the demo-rollup/benches folder and supports the following commands
* **make criterion** - generates the criterion benchmark using rollup_bench.rs
* **make basic** - supports the coarse grained timers (getting the TPS) using rollup_coarse_measure.rs
* **make prometheus** - runs rollup_coarse_measure.rs but instead of aggregating std::time directly and printing in a table, it outputs a json containing histogram metrics populated by the script
* **make flamegraph** - runs `cargo flamegraph`. On mac this requires `sudo` permissions. The script ensures some cleanup and to err on the side of caution, it deletes the `sovereign/target` folder since new artifacts can be owned by root

The Makefile supports setting number of blocks and transactions per block using SOV_BENCH_BLOCKS and SOV_BENCH_TXNS_PER_BLOCKS env vars. Defaults are 100 blocks and 10,000 transactions per block when using the Makefile

![Flamegraph](flamegraph_sample.svg)
