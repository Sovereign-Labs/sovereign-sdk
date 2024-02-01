# Getting Started

## Setting up your IDE

To get Rust Analyzer (or other language plugins) to work with Risc0, you'll need to set up the risc0 stdlib binaries in your sysroot.

0. `cd methods/guest`. All future instructions assume that you're at the root of the guest crate.
1. Use `download_std.rs` to fetch the modified std lib code. One easy way to run this code is to rename it to `build.rs` and then
   `cargo build`. The downloaded code will be stored in a directory called "riscv-guest-shim", which should be placed in the
   `methods/guest` directory.
2. Build `.rlib` binaries from the modified stdlib. To build the standard library, use the command below (replacing the path).
   To minimize extraneous code, you may want to comment out the dependencies section of `Cargo.toml` and the body of
   `rollup.rs` before continuing.

```
__CARGO_TESTS_ONLY_SRC_ROOT="/path/to/riscv-guest-shim/rust-std" cargo run -Zbuild-std=core,alloc,proc_macro,panic_abort,std -Zbuild-std-features=compiler-builtins-mem --test --release
```

The output of this build will be stored in `target/riscv32im-risc0-zkvm-elf/release/deps`.

Identify the `SYSROOT` from your default rust compiler using the command `rustc --print sysyroot`. Make a new directory
`lib/rustlib/riscv32im-risc0-zkvm-elf/lib` under your sysroot, and copy all of the stdlib `*.rlib` files into it. Now, your
toolchain will recognize `riscv32im-risc0-zkvm-elf` as an installed target.

If you encounter any problems, try re-running above `cargo run -Zbuildstd ...` command without the `--test` flag, copy any additional
`.rlib`s into the `sysroot/lib/rustlib/riscv32im-risc0-zkvm-elf/lib` directory.
