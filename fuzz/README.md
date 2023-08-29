# LLVM's libFuzzer

This implementation is built upon [libfuzzer-sys](https://crates.io/crates/libfuzzer-sys). For more information, check [LLVM](https://llvm.org/docs/LibFuzzer.html) documentation.

## Build

To build the fuzz target, run the following command:

```sh
make build
```

You can build in release mode via:

```sh
make build ARGS=--release
```

Some special parameters are required to build the fuzz target. As example, let's build the `namespace_group_from_b64` fuzz target:

```sh
cargo rustc --bin namespace_group_from_b64 \
    --manifest-path fuzz/Cargo.toml -- \
    -C debuginfo=full \
    -C debug-assertions \
    -C passes='sancov-module' \
    -C llvm-args='-sanitizer-coverage-level=3' \
    -C llvm-args='-sanitizer-coverage-inline-8bit-counters' \
    -Z sanitizer=address
```

We don't default these options as they depend on the `rustc` version and might change in the future. For the list of available targets, check [Cargo.toml](./fuzz/Cargo.toml) under the `bin` section. We are currently not using optimized binaries as it might impact on how rocksdb is built. If you want to activate optimization, add `--release` after `rustc`.

Unfortunately, rustc doesn't support the `--bins` argument to build multiple binaries with custom compiler directives. We have to build every target individually. Below is a convenience [sed](https://www.gnu.org/software/sed/) script to build all targets.

```sh
for t in `sed -n '/^\[\[bin\]\]/,/^$/ { /name\s*=\s*"\(.*\)"/s//\1/p }' fuzz/Cargo.toml` ; do cargo rustc --bin $t --manifest-path fuzz/Cargo.toml -- -C debuginfo=full -C debug-assertions -C passes='sancov-module' -C llvm-args='-sanitizer-coverage-level=3' -C llvm-args='-sanitizer-coverage-inline-8bit-counters' -Z sanitizer=address ; done
```

## Run

Here is a sample command to fuzz a `namespace_group_from_b64`:

```sh
make run TARGET=namespace_group_from_b64
```

To run in release mode:

```sh
make run TARGET=namespace_group_from_b64 PROFILE=release
```

To list the available targets, run:

```sh
make targets
```

Once built, you can run the targets under the `fuzz/target/<profile>` directory.

```sh
./fuzz/target/debug/namespace_group_from_b64
```

It will run the fuzz until you interrupt the command (i.e. `CTRL-C`), and will record crashes under `fuzz/artifacts/*/crash-*`. If you find a crash, please report a new [bug](https://github.com/Sovereign-Labs/sovereign-sdk/issues/new?assignees=&labels=&projects=&template=bug_report.md&title=).
