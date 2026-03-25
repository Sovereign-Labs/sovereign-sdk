---
name: rust-compile-times
description: >
  Analyze, diagnose, and reduce Rust crate compilation times in a workspace.
  Use this skill whenever the user mentions slow Rust builds, wants to profile
  compile times, asks which crates or dependencies are slow, wants to visualize
  the build graph, or asks about reducing compile times in a Cargo workspace.
  Triggers include: "why is my Rust build slow", "analyze compile time",
  "which crate takes the longest", "cargo build is slow", "speed up Rust
  compilation", "bottleneck in my workspace", or any mention of cargo --timings,
  cargo-llvm-lines, or cargo-bloat in the context of build performance.
---

# Rust Compile Time Analysis

## Overview

Rust compilation is slow by design (monomorphization, LLVM backend, borrow checker).
This skill gives you a systematic process to **measure → identify → fix** compile
time bottlenecks in a Cargo workspace.

Work through the levels in order. Stop when you've found and resolved your bottleneck —
deeper levels are more surgical but also more invasive.

---

## Scripts

Two helper scripts are bundled in `scripts/`. Run them from the workspace root.

| Script | Purpose |
|--------|---------|
| `bash scripts/audit.sh` | Full audit: workspace structure, dep count, timed build, proc-macros, llvm-lines. Prints a structured report for Claude to read. |
| `python3 scripts/parse_timings.py` | Parse `target/cargo-timings/cargo-timing-*.json` (or `.html`) and print a sorted table of per-crate compile durations. Run after any `cargo build --timings`. |
| `bash scripts/binary_sizes.sh` | Report test binary sizes and optional CGU breakdown (`--cgu` flag) for `sov-demo-rollup`. |
| `bash scripts/measure_incremental.sh` | Measure incremental rebuild time. Supports `--file` and `--test` flags to target specific files/tests. |

**When helping a user, ask them to run `bash scripts/audit.sh` first.** The output
gives you everything needed to diagnose the bottleneck in one pass.

---

## Level 0 — Prerequisites

Before measuring, confirm the environment:

```bash
rustc --version          # Note toolchain; nightly unlocks extra flags
cargo --version
cargo tree --depth 1     # Get a feel for direct deps
```

Ensure you're doing a **clean build** for accurate results:

```bash
cargo clean
```

> Incremental builds hide the true cost of a crate. Always `cargo clean` before
> a timing run you intend to compare against.

---

## Level 1 — Fast Scan with `cargo --timings` (Start Here)

This is built into stable Cargo and requires zero extra installs.

```bash
cargo build --timings
# OR for a workspace-wide view:
cargo build --workspace --timings
```

Output: `target/cargo-timings/cargo-timing.html`

**What to look for:**

| Signal | Meaning |
|--------|---------|
| A tall red "Waiting" spike | Many crates blocked on one dependency — fix that one crate |
| Long horizontal green bars | A single crate takes disproportionate time |
| Low parallelism (few concurrent green bars) | Critical path is too long; consider splitting |
| Large codegen phase (visible per unit) | Monomorphization or macro explosion |

**Actionable rule:** The crate with the widest bar on the critical path is your
primary target. Note its name and proceed to Level 2 or 3.

---

## Level 2 — Dependency Weight

`cargo-bloat --time` was **removed** in v0.12.0 — the flag no longer exists.
For per-crate compile time you have two options:

**Option A — Parse the timings JSON directly (recommended):**
```bash
cargo build --timings              # generates target/cargo-timings/cargo-timing*.json
python3 scripts/parse_timings.py   # reads the JSON, prints a sorted table
```

**Option B — Use the `--message-format` output and time manually:**
```bash
cargo build --message-format=json 2>&1 \
  | python3 -c "
import sys,json
for line in sys.stdin:
    try:
        m = json.loads(line)
        if m.get('reason') == 'compiler-artifact':
            print(m['target']['name'], m.get('fresh', False))
    except: pass
"
```

`cargo-bloat` remains useful for **binary size** analysis (not compile time):
```bash
cargo install cargo-bloat
cargo bloat --release --crates    # which deps bloat the final binary
```

Also run `cargo tree` to understand which of your deps pull in heavy transitive
dependencies:

```bash
cargo tree -d              # Show duplicate crates (different versions)
cargo tree -p <slow-crate> # Trace why a crate is in the graph
```

**Targets to eliminate or replace:**

- Duplicate crate versions — unify via `[patch]` or `cargo update`
- Heavy proc-macro crates (`serde`, `tokio`, `async-trait`) pulled into many
  crates — make them optional features where possible
- Unused dependencies — check with `cargo machete` or `cargo udeps`

---

## Level 3 — Generic Bloat with `cargo-llvm-lines`

Install:

```bash
cargo install cargo-llvm-lines
```

Run on a specific crate (generics are monomorphized into the crate that uses them,
not the crate that defines them):

```bash
cargo llvm-lines -p <your-crate>
# Full workspace view via LTO (slower but holistic):
CARGO_PROFILE_RELEASE_LTO=fat cargo llvm-lines --release
```

**Reading the output:**

```
Lines    Copies  Function name
30737    1107    (TOTAL)
 1395      83    core::ptr::drop_in_place    ← high copies = monomorphization
  760       2    alloc::slice::merge_sort
```

- **High Lines + High Copies**: a generic function instantiated too many times.
  Refactor to push non-generic work behind a non-generic helper.
- **High Lines + Low Copies**: one big function — consider splitting or using
  `#[inline(never)]`.

**Common fixes:**

```rust
// Bad: whole function is generic
fn process<T: Trait>(x: T) { /* 100 lines */ }

// Good: only the thin wrapper is generic
fn process<T: Trait>(x: T) { process_inner(&x as &dyn Trait) }
fn process_inner(x: &dyn Trait) { /* 100 lines */ }
```

---

## Level 4 — Proc-Macro Cost (`-Zmacro-stats`, nightly only)

Proc-macros (especially `serde::Derive`, `thiserror`, `async-trait`) can generate
enormous amounts of code silently.

```bash
# For a single crate:
RUSTFLAGS="-Zmacro-stats" cargo +nightly check -p <crate>
```

The output shows code generated (in lines) per macro invocation. If a macro is
generating more code than you wrote by hand, it's worth investigating alternatives
or restricting its use.

**Common offenders and alternatives:**

| Macro | Cheaper alternative |
|-------|-------------------|
| `serde::Serialize/Deserialize` on shared types | Make serde an optional feature |
| `#[async_trait]` | Use `impl Trait` in return position (stable in recent Rust) |
| `thiserror` everywhere | Plain `impl Display` for leaf error types |
| `derive(Debug)` on large types | Manual or no impl in inner crates |

---

## Level 5 — Deep Profile with `-Zself-profile` (nightly only)

When you need to know exactly where rustc spends its time inside a crate:

```bash
RUSTFLAGS="-Zself-profile" cargo +nightly build -p <crate>
# Produces .mm_profdata files in current directory
```

Visualize with `measureme`:

```bash
cargo install measureme --features="bin"
summarize summarize <profile-file>.mm_profdata
# Or open as a Chromium trace:
crox <profile-file>.mm_profdata
# Then open chrome://tracing and load the JSON
```

Look for:
- `typeck` / `mir_borrowck` time → complex types or trait bounds
- `codegen` time → too many CGUs or large monomorphized functions
- `LLVM` time → reduce IR generated (see Level 3)

---

## Level 6 — Linker Bottleneck

Often overlooked: linking can dominate incremental build time.

```bash
# Measure link time specifically:
cargo clean
cargo +nightly rustc --bin <binary> -- -Ztime-passes 2>&1 | grep "^time:"
```

**Fast linker options** (add to `.cargo/config.toml`):

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]   # mold: fastest
# OR:
rustflags = ["-C", "link-arg=-fuse-ld=lld"]     # lld: good, ships with LLVM
```

Install mold: `sudo apt install mold` (Debian/Ubuntu) or build from source.

---

## Workspace-Wide Optimization Checklist

After identifying bottlenecks, apply these in order of impact:

1. **Split the critical-path crate** — if one crate blocks everything, break
   it into a smaller `*-core` crate that downstream crates depend on, and a
   larger `*-impl` crate built in parallel.

2. **Use `cargo check` in development** — doesn't produce codegen output,
   ~3–5× faster than `cargo build` for iteration.

3. **Enable parallel front-end** (nightly):
   ```toml
   # .cargo/config.toml
   [build]
   rustflags = ["-Z", "threads=8"]
   ```

4. **Set `codegen-units` for dev profile**:
   ```toml
   # Cargo.toml
   [profile.dev]
   codegen-units = 256   # default is 256 for dev; set higher to reduce codegen per CGU
   ```

5. **Use `sccache` for CI caching**:
   ```bash
   cargo install sccache
   export RUSTC_WRAPPER=sccache
   ```

6. **Switch to `cranelift` backend for dev** (much faster codegen, no optimization):
   ```toml
   # .cargo/config.toml
   [unstable]
   codegen-backend = true

   [profile.dev]
   codegen-backend = "cranelift"
   ```
   Requires nightly. Not suitable for release builds.

---

## Interpreting Results: Decision Tree

```
cargo build --timings
        │
        ├─ One crate dominates? ──► cargo-llvm-lines -p <that-crate>
        │                           ↳ High copies? → de-monomorphize
        │                           ↳ High lines?  → macro-stats, split crate
        │
        ├─ Many small crates blocked? ──► cargo-bloat --time
        │                                ↳ Heavy transitive dep? → optional feature / replace
        │                                ↳ Duplicate versions?   → cargo update / [patch]
        │
        └─ Build fast but link slow? ──► Switch to mold or lld linker
```

---

## References

- Cargo timings docs: https://doc.rust-lang.org/cargo/reference/timings.html
- Rust Performance Book (compile times): https://nnethercote.github.io/perf-book/compile-times.html
- corrode: Tips for Faster Rust Compile Times: https://corrode.dev/blog/tips-for-faster-rust-compile-times/
- cargo-llvm-lines: https://github.com/dtolnay/cargo-llvm-lines
- mold linker: https://github.com/rui314/mold
- measureme / self-profile: https://github.com/rust-lang/measureme