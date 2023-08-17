# Prover Benchmarks
* For benchmarking the prover, we measure the number of risc0 vm cycles for each of the major functions.
* The reason for using the cycles is the assumption that proving works off a cycles/second (KHz, MHz) based on the hardware used

## Running the bench
* From sovereign-sdk
```
$ cd examples/demo-prover/host/benches
$ cargo bench --features bench --bench prover_bench
```

## Methodology
* We have `cycle_tracker` macro defined which can be used to annotate a function in zk that we want to measure the cycles for
* The `cycle_tracker` macro is defined at `sovereign-sdk/zk-cycle-util`
```rust
    #[cfg_attr(all(target_os = "zkvm", feature="bench"), cycle_tracker)]
    fn begin_slot(&mut self, witness: Self::Witness) {
        self.checkpoint = Some(StateCheckpoint::with_witness(
            self.current_storage.clone(),
            witness,
        ));
    }
```
* The method we use to track metrics is by registering the `io_callback` syscall when creating the risc0 host.
```
pub fn get_syscall_name_handler() -> (SyscallName, fn(&[u8]) -> Vec<u8>) {
    let cycle_string = "cycle_metrics\0";
    let bytes = cycle_string.as_bytes();
    let metrics_syscall_name = unsafe {
        SyscallName::from_bytes_with_nul(bytes.as_ptr())
    };

    let metrics_callback = |input: &[u8]| -> Vec<u8> {
        {
            let met_tuple = deserialize_custom(input);
            add_value(met_tuple.0, met_tuple.1);
        }
        vec![]
    };

    (metrics_syscall_name, metrics_callback)

}

#[cfg(feature = "bench")]
{
    let (metrics_syscall_name, metrics_callback) = get_syscall_name_handler();
    default_env.io_callback(metrics_syscall_name, metrics_callback);
}
```
* The above allows us to use `risc0_zkvm::guest::env::send_recv_slice` which lets the guest pass a slice of raw bytes to host and get back a vector of bytes
* We use it to pass cycle metrics to the host
* Cycles are tracked by the macro which gets a cycle count at the beginning and end of the function
```rust
let before = risc0_zkvm::guest::env::get_cycle_count();
let result = (|| #block)();
let after = risc0_zkvm::guest::env::get_cycle_count();
```
* We feature gate the application of the macro `cycle_tracker` with both the target_os set to `zkvm` and the feature flag `bench`
* The reason for using both is that we need conditional compilation to work in all cases
* For the purpose of this profiling we run the prover without generating the proof

## Input set
* Unlike demo-prover it's harder to generate fake data since all the proofs and checks need to succeed. 
* This means the DA samples, hashes, signatures etc need to succeed
* To make this easier we use a static input set consisting of 3 blocks
  * we avoid using empty blocks because they skew average metrics
  * we have 3 blocks
    * block 1 -> 1 blob containing 1 create token transaction
    * block 2 -> 1 blob containing 1 transfer transaction
    * block 3 -> 1 blob containing 2 transfer transactions
* This dataset is stored at `demo-prover/benches/blocks.hex`
* The dataset can be substituted with another valid dataset as well from Celestia (TBD: automate parametrized generation of blocks.hex)
* We can run this on different kinds of workloads to gauge the efficiency of different parts of the code

## Result
* Standard hash function patched with risc0/rust_crypto
* Signature verification currently NOT patched (TBD)
* Signature verification takes about 60% of the total cycles
```
Block stats

+------------------------------------------+---+
| Total blocks                             | 3 |
+------------------------------------------+---+
| Blocks with transactions                 | 3 |
+------------------------------------------+---+
| Number of blobs                          | 3 |
+------------------------------------------+---+
| Total number of transactions             | 4 |
+------------------------------------------+---+
| Average number of transactions per block | 1 |
+------------------------------------------+---+

Cycle Metrics

+-------------------------+----------------+-----------+
| Function                | Average Cycles | Num Calls |
+-------------------------+----------------+-----------+
| Cycles per block        | 6935250        | 3         |
+-------------------------+----------------+-----------+
| apply_slot              | 6433166        | 3         |
+-------------------------+----------------+-----------+
| verify                  | 3965858        | 4         |
+-------------------------+----------------+-----------+
| end_slot                | 514929         | 3         |
+-------------------------+----------------+-----------+
| validate_and_commit     | 496189         | 3         |
+-------------------------+----------------+-----------+
| verify_relevant_tx_list | 277438         | 3         |
+-------------------------+----------------+-----------+
| begin_slot              | 4683           | 3         |
+-------------------------+----------------+-----------+
```

## Custom annotations
* We can also get finer grained information by annotating low level functions, but the process for this isn't straightforward. 
* For code that we control, it's as simple as adding the `cycle_tracker` annotation to our function and then feature gating it (not feature gating it causes compilation errors)
* For external dependencies, we need to fork and include a path dependency locally after annotating
* We did this for the `jmt` jellyfish merkle tree library to measure cycle gains when we use the risc0 accelerated sha function vs without
* We apply the risc0 patch in the following way in demo-prover/methods/guest/Cargo.toml
```yaml
[patch.crates-io]
sha2 = { git = "https://github.com/risc0/RustCrypto-hashes", tag = "sha2/v0.10.6-risc0" }
```
* Note that the specific tag needs to be pointed to, since master and other branches don't contain acceleration

## Accelerated vs Non-accelerated libs
* Accelerated and risc0 optimized crypto libraries give a significant (nearly 10x) cycle gain
* With sha2 acceleration
```
=====> hash: 1781
=====> hash: 1781
=====> hash: 1781
=====> hash: 1781
=====> hash: 1781
```
* Without sha2 acceleration
```
=====> hash: 13901
=====> hash: 13901
=====> hash: 13901
=====> hash: 13901
=====> hash: 13901
```
* Overall performance difference when using sha acceleration vs without for the same dataset (3 blocks, 4 transactions) as described above
* With sha acceleration
```
+-------------------------+----------------+-----------+
| Function                | Average Cycles | Num Calls |
+-------------------------+----------------+-----------+
| Cycles per block        | 6944938        | 3         |
+-------------------------+----------------+-----------+
| validate_and_commit     | 503468         | 3         |
+-------------------------+----------------+-----------+
| verify_relevant_tx_list | 277092         | 3         |
+-------------------------+----------------+-----------+
Total cycles consumed for test: 20834815
```
* Without sha acceleration
```
+-------------------------+----------------+-----------+
| Function                | Average Cycles | Num Calls |
+-------------------------+----------------+-----------+
| Cycles per block        | 8717567        | 3         |
+-------------------------+----------------+-----------+
| validate_and_commit     | 1432461        | 3         |
+-------------------------+----------------+-----------+
| verify_relevant_tx_list | 966893         | 3         |
+-------------------------+----------------+-----------+
Total cycles consumed for test: 26152702
```
* There's an overall efficiency of 6 million cycles in total for 3 blocks. 
* Keep in mind that the above table shows average number of cycles per call, so they give an efficiency per call, but the "Total cycles consumed for test" metric at the bottom shows total for 3 blocks