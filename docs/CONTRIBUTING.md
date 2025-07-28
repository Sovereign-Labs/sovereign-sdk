# Contributing to Sovereign SDK

We welcome contributions from the entire community to enhance the Sovereign SDK. Feel free to submit suggestions, report bugs, propose pull requests, or provide valuable feedback following the guidelines outlined in this document.

## Setting up your editor

Most contributors use VS Code with the [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) extension. If you do as well, we suggest copying the contents of `.vscode/settings.default.json` to `.vscode/settings.json` to enable our recommended settings.

## Setting up your machine

Run this:

```sh
make install-dev-tools
```

## Submitting bug reports and feature requests

When submitting a bug report or requesting assistance, please provide sufficient information for us to reproduce the observed behavior. For guidance on supplying such details, refer to the instructions on creating a [Minimal, Comprehensive, and Verifiable Example].

[Minimal, Comprehensive, and Verifiable Example]: <https://stackoverflow.com/help/minimal-reproducible-example>

For feature requests, clearly articulate the problem your proposed addition aims to address, propose potential ways the SDK could facilitate this solution, consider alternative approaches, and acknowledge any potential drawbacks.

## Running the test suite

We encourage you to run the test suite locally prior to submitting a pull request with your proposed modifications. Should any tests fail, it is usually more efficient to address the issues at the local level instead of waiting for the CI servers to execute the tests on your behalf.

```sh
# Check for code style
make lint
# Run the test suite
make test
# Check if all targets are building
make check-features
```

## Code lints

#### Safe arithmetic

To ensure the integrity and reliability of our codebase, we prioritize safe arithmetic practices. This approach is essential for preventing silent failures during optimized builds, where overflow checks are disabled. The following guidelines outline our arithmetic operations:

- Saturated addition and multiplication:
  - Memory-bound operations:
    - Addition/multiplication of memory-addressable values should not result in overflows under normal circumstances due to their size discrepancies - for example, u32 or u64 versus allocated bytes length.
  - Gas usage and price calculation:
    - Although gas usage normally does not reach integer bounds, its arithmetic operations must be error-free to accurately calculate the gas funds that the transaction payer intends to withdraw.

- Checked addition and multiplication:
  - Balances and total supply:
    - Balances and total supply should not exceed maximum integer size under normal conditions. Any such occurrence may indicate corrupted state or flawed logic.

- Checked subtraction and division:
  - Memory-bound operations:
    - Subtraction/division of memory-bound values, intended to calculate offsets, must not overflow. Overflow is usually a result of corrupt states or incorrect logic.
  - Height and nonce operations:
    - Similarly, subtraction/division of height and nonce values, meant to compute offsets, should not result in overflows. Any overflow here is typically due to faulty state or illogical processing.

- Wrapped addition and multiplication:
  - Height and nonce operations:
    - Addition/multiplication of height and nonce values, linked to network usage, can be wrapped when overflows occur. While such usage may indicate excessive network activity, wrapping does not pose any immediate safety concerns as the prior state remains consistent.

## Conduct

We follow the [Rust Code of Conduct].

[Rust Code of Conduct]: https://www.rust-lang.org/policies/code-of-conduct
