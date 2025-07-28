# Example Programs for SP1

This directory contains example programs for SP1 which are used for testing, and benching the performance
of the SP1 zkVM.

The [build.rs](./build.rs) file contains a script which builds the SP1 zkVM and the programs that are used to
benchmark the performance of the SP1 zkVM.

If you want to manually build the programs, you can do so within the `guest-celestia` or `guest-mock` directory by installing the SP1 `cargo prove` toolchain and then running:

```shell
cargo prove build
```