# AGENTS.md

This file provides guidance to AI Agents when working with code in this repository.

## Project Overview

Sovereign SDK is a Rust toolkit for building rollups with real-time soft-confirmations, high performance (30k+ UOPS), and pluggable data availability (Celestia, Bitcoin) and zkVM (Risc0, SP1) adapters.

## Build Commands

```bash
make lint                   # Run all linters (fmt, clippy, zepter, dylint)
make lint-fix               # Auto-fix linting issues
make test                   # Run tests with nextest
make check-features         # Verify all feature combinations compile
make mini-ci                # Full pre-submission checks
make install-dev-tools      # Install all development dependencies
```

Instead of running `make build` use `make lint` or `cargo check --all-features` for faster feedback loops.

### Running Tests

```bash
cargo nextest run <test_name>
cargo nextest run -p <package_name> <test_name>
```

- Prefer running tests with `-p` to reduce rebuild times.
- Always use `cargo nextest` over regular `cargo test`. Cargo nextest runs each test in a separate process which gives isolation of environment variables.
- This project has strict testing practices. It means that tests on base branch(`dev`) are always passing. 
  Any consistent failure in the branch is certainly introduced in the branch.

### Debugging

- Logs: add `sov_test_utils::initialize_logging()` and run test like this. `RUST_LOG=debug cargo nextest run -p <pkg> <test>`. If there's too much information, adjust `RUST_LOG` to only emit logs from relevant modules.
- For DB issues: Check `sov-db` traces with `RUST_LOG=sov_db=trace`
- Feel free to add println! or logs if it helps. 

### Environment Variables

- `PROPTEST_CASES=50` - Faster local proptest runs (CI uses more)
- `SKIP_GUEST_BUILD=1` - Skip risc0 guest builds during development
- `SP1_SKIP_PROGRAM_BUILD=1` - Skip SP1 program builds during development
- `SOV_TEST_SKIP_DOCKER=1` - Skip docker-based tests, useful for local development without docker

Always use `SKIP_GUEST_BUILD=1` unless performing specific ZK related changes, because it massively speeds up compilation time.

## Architecture

### Key Design Principles

- **Modularity**: Pluggable DA(Data Availability) layers, storage backends, and ZK systems
- **Type Safety**: Strong typing with Rust's type system, because it allows checking correctness at compile time.
- **Determinism**: All operations deterministic for ZK proving. No HashMap/HashSet iteration order, no system time, no random. Breaks ZK proofs.
- **Gas Metering**: Automatic tracking of compute and storage costs
- **State Isolation**: Each module's state is namespaced. Modules cannot directly access other modules' state; use hooks or explicit dependencies / methods

### Four Major Layers

1. **Rollup Interface** (`crates/rollup-interface/`) - Core traits defining rollup behavior
2. **Module System** (`crates/module-system/`) - Framework for building modules
3. **Adapters** (`crates/adapters/`) - DA and zkVM integrations
4. **Full Node** (`crates/full-node/`) - Node infrastructure

Delegate to subagents & AGENTS.md in those directories for more details.

### Bindings

- `./typescript/`: TypeScript bindings for serialization, transaction building, and RPC client
- `./python/`: Python bindings for simple transaction building & serialization in python


## Code Quality Requirements

Avoid over-engineering solutions. Prioritise clarity and maintainability. Don't prematurely add code or features that aren't needed.

### Philosophy

This codebase will outlive you. Avoid shortcuts that create long-term debt.
- Validate the plan and downstream impact; small mistakes can propagate.
- Take time to understand the question before changing code.
- Keep docs/comments in sync with behavior.
- Improve logging around changed areas if it helps future debugging.

### Non-Determinism

- Always avoid non-deterministic code paths in modules and core logic, because non-determinism breaks consensus code and ZK proofs for the light clients.
- For tests, avoid using `sleep`, prefer deterministic detection of expected changes.

### Safe Arithmetic

- Use checked/saturating operations for balances, gas calculations, and memory-bound operations
- No float arithmetic (lint denial)

### Feature Gates

- `native` feature: Code not executed in zk proofs
- Most crates use `default-features = false` for zkVM compatibility

### Linting

- Clippy with custom configuration
- Zepter for feature-gate consistency
- Dylint for Sovereign-specific lints
- Nightly rustfmt (use `rustfmt.nightly.toml`)

## Communication

- Always favour clear and concise over verbose. Add more detail when asked
- Always state if you are guessing or making an assumption
- Always link to relevant files, lines, or documentation when referencing code or answering questions
    - Prefer style `[filename:line_number]` for inline references
    - Prefer style `[filename]` for general references

## Code Review

- Base branch for this project is `dev`.
- Focus on business logic, correctness and code quality which has the highest business impact. Ignore untracked files.
- Ignore uncommitted changes in Cargo.toml, if it removes `default-members`. This is expected pattern with `cargo switcheroo`

# Agent working agreement

## Default operating mode: Minimal Patch Mode

Unless the user explicitly asks for a refactor, redesign, or broader cleanup, operate in Minimal Patch Mode.

Rules:
- Make the smallest diff that satisfies the request.
- Preserve the requested approach, constraints, and test strategy.
- Prefer editing existing code over introducing new abstractions.
- Do not fix adjacent issues in the same PR.
- Put unrelated ideas under "Follow-ups", not in the diff.

## Plan Lock

An approved plan is binding.

Rules:
- Implement the approved plan, not a "better" plan.
- **Do not silently replace requested techniques**, test strategies, or constraints.
- **Do not widen scope** because a broader change seems cleaner.
- Do not drop explicit user requirements from earlier discussion just because they were omitted from a later summary.
- If the plan omitted a prior explicit requirement, repair the plan before coding.

If you believe the approved plan is flawed, stop and report:
1. the exact conflict or new evidence
2. the smallest change to the plan that would fix it
3. zero implementation changes beyond investigation

Any deviation from the approved plan must be called out explicitly under:
`Deviations from approved plan: ...`
**Silent deviations are not allowed.**

## Scope budget

Default budget **unless explicitly approved otherwise**:
- at most 4 files changed
- at most 200 added non-test lines
- no new dependencies
- no public API changes
- no file moves or broad renames
- no opportunistic refactors
- no formatting-only churn
- no test rewrites unless explicitly requested

If the task cannot be completed within this budget, stop and propose the smallest split.

## Review behavior

Reviewer direction wins over agent preference.

Rules:
- When asked to reduce scope, reduce scope.
- Do not argue for the broader patch after the reviewer asks for a smaller PR.
- You may note one concise technical risk or tradeoff, then comply.
- Remove incidental changes instead of defending them.