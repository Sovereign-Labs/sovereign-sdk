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
* To make this easier we use a static input set consisting of 8 blocks
  * 0,1,2 - empty blocks
  * 3 - contains 1 token creation transactions
  * 4 - contains 1 transfer transaction
  * 5 - contains 2 transfer transactions
  * 6,7 - empty blocks
* This dataset is stored at `demo-prover/benches/blocks.hex`
* The dataset can be substituted with another valid dataset as well from Celestia (TBD: automate parametrized generation of blocks.hex)
* 

## Result
```
Block stats

+------------------------------------------+---+
| Total blocks                             | 8 |
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
| Cycles per block        | 6664105        | 3         |
+-------------------------+----------------+-----------+
| apply_blob              | 5844454        | 3         |
+-------------------------+----------------+-----------+
| verify                  | 3921431        | 4         |
+-------------------------+----------------+-----------+
| end_slot                | 452223         | 3         |
+-------------------------+----------------+-----------+
| validate_and_commit     | 440060         | 3         |
+-------------------------+----------------+-----------+
| verify_relevant_tx_list | 331109         | 3         |
+-------------------------+----------------+-----------+
| begin_slot              | 3599           | 3         |
+-------------------------+----------------+-----------+

Total cycles consumed for test: 19992315
```