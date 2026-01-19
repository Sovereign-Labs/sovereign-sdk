# Coding Guidelines

## Quick Reference

- **Dependencies**: Use `default_features = false`; be conservative adding new deps
- **Error handling**: Don't use `if let Ok` without a comment explaining why it's safe
- **Patches**: Don't use `patch.crates-io` for SDK crates; use git references instead
- **Feature gates**: Don't feature-gate imports; use fully qualified paths in feature-gated blocks
- **Assertions**: Use `assert_eq!(actual, expected)` over `matches!`
- **Test scope**: One concern per test; don't add unrelated assertions
- **Tempdir**: Declare at top of test so it outlives async/threaded operations
- **Docs**: Use `#[allow(missing_docs)]` only when documentation would be redundant

---

## Implementation

### Dependencies

**When**: Adding a new crate dependency to `Cargo.toml`.

**Rule**:
- DON'T: Add dependencies without considering the trade-offs
- DON'T: Use default features unless specifically needed
- DO: Use `default_features = false` and enable only required features
- DO: Be extra cautious with macro/proc-macro dependencies (build scripts can rewrite code)

**Why**: Default features pollute the dependency tree, increase compilation time, and slow down IDE responsiveness.

**Example**:

```toml
# BAD:
[dependencies]
serde = "1.0"

# GOOD:
[dependencies]
serde = { version = "1.0", default-features = false, features = ["alloc"] }
```

---

### Error Handling with `if let Ok`

**When**: You encounter or are about to write `if let Ok(x) = ...` pattern.

**Rule**:
- DON'T: Use `if let Ok` without documenting why swallowing the error is acceptable
- DO: Handle errors explicitly, or add a comment explaining why ignoring is safe

**Why**: This pattern silently swallows errors, making debugging significantly harder.

**Example**:

```rust
// BAD: Error is silently swallowed - if this fails, we have no idea why
if let Ok(value) = parse_config(path) {
    use_config(value);
}

// GOOD: Handle the error explicitly
match parse_config(path) {
    Ok(value) => use_config(value),
    Err(e) => tracing::warn!("Failed to parse config: {e}, using defaults"),
}

// GOOD: If ignoring is intentional, document why
// We intentionally ignore errors here because the cache is optional
// and we fall back to fetching from the network
if let Ok(cached) = read_cache(key) {
    return cached;
}
```

---

### SDK Crate Patches

**When**: You need to use a modified version of an SDK crate.

**Rule**:
- DON'T: Use `[patch.crates-io]` in `Cargo.toml` for SDK crates
- DO: Use git references directly in the dependency declaration

**Why**: Patches are only applied to the root `Cargo.toml`. Users of the SDK won't get the patch, leading to version mismatches and confusing bugs.

**Example**:

```toml
# BAD: Patch won't apply for SDK users
[patch.crates-io]
some-sdk-crate = { git = "https://github.com/org/repo", branch = "fix" }

# GOOD: Git reference travels with the dependency
[dependencies]
some-sdk-crate = { git = "https://github.com/org/repo", branch = "fix" }
```

---

## Feature Gating

### Avoid Feature-Gated Imports

**When**: Writing code that is conditionally compiled with `#[cfg(feature = "...")]`.

**Rule**:
- DON'T: Feature-gate `use` statements separately from the code that uses them
- DO: Use fully qualified paths inside feature-gated blocks

**Why**: Minimizes feature surface area and keeps related code together.

**Example**:

```rust
// BAD: Two separate feature gates, import separated from usage
#[cfg(feature = "native")]
use bar::Bar;

#[cfg(feature = "native")]
impl<Foo: Bar> Baz<Foo> {
    fn do_something(&self) { /* ... */ }
}

// GOOD: Single feature gate, fully qualified path
#[cfg(feature = "native")]
impl<Foo: bar::Bar> Baz<Foo> {
    fn do_something(&self) { /* ... */ }
}
```

---

## Testing

### Prefer `assert_eq!` Over `matches!`

**When**: Writing test assertions that compare values.

**Rule**:
- DON'T: Use `matches!` for equality checks
- DO: Use `assert_eq!` with actual value on the left, expected on the right

**Why**: `assert_eq!` prints both values on failure; `matches!` only tells you it didn't match.

**Example**:

```rust
// BAD: On failure, you only know it didn't match
assert!(matches!(result, Ok(42)));

// GOOD: On failure, you see both actual and expected values
assert_eq!(result, Ok(42));

// Output on failure shows both sides:
// thread 'test' panicked at 'assertion failed: `(left == right)`
//   left: `Ok(41)`,
//  right: `Ok(42)`'
```

---

### Assertion Argument Order

**When**: Writing any `assert_eq!` statement.

**Rule**:
- DO: Place `actual` on the left, `expected` on the right: `assert_eq!(actual, expected)`

**Why**: Consistent ordering makes failure output predictable. "left" is always what you got, "right" is always what you wanted.

**Example**:

```rust
// BAD: Inconsistent with project convention
assert_eq!(expected_balance, account.balance());

// GOOD: Actual first, expected second
assert_eq!(account.balance(), expected_balance);

// Failure output is now intuitive:
// left: `100`,   <- what we got (actual)
// right: `200`   <- what we expected
```

---

### Human-Readable Assertion Messages

**When**: The assertion context isn't obvious from the test name or surrounding code.

**Rule**:
- DO: Add a descriptive message explaining what failed from a domain perspective

**Why**: Helps developers understand failures faster without tracing through code.

**Example**:

```rust
// GOOD: Message provides domain context
assert_eq!(
    account.balance(),
    initial_balance + deposit,
    "Balance should increase by exact deposit amount after transfer"
);

// Failure output now includes context:
// assertion failed: `(left == right)`
//   left: `150`,
//  right: `200`: Balance should increase by exact deposit amount after transfer
```

---

### One Concern Per Test

**When**: You're tempted to add an extra assertion to an existing test.

**Rule**:
- DON'T: Add assertions unrelated to the test's primary purpose
- DO: Extract common setup and create separate focused tests

**Why**: Growing test scope causes "unrelated test failing" confusion when code changes. Focused tests pinpoint exactly what broke.

**Example**:

```rust
// BAD: Test is checking multiple unrelated things
#[test]
fn test_transfer() {
    let mut bank = setup_bank();
    bank.transfer(alice, bob, 100);

    assert_eq!(bank.balance(alice), 900);
    assert_eq!(bank.balance(bob), 100);
    assert_eq!(bank.total_supply(), 1000);  // Unrelated to transfer logic
    assert_eq!(bank.transaction_count(), 1); // Unrelated to transfer logic
}

// GOOD: Separate focused tests
#[test]
fn test_transfer_updates_balances() {
    let mut bank = setup_bank();
    bank.transfer(alice, bob, 100);

    assert_eq!(bank.balance(alice), 900);
    assert_eq!(bank.balance(bob), 100);
}

#[test]
fn test_transfer_increments_transaction_count() {
    let mut bank = setup_bank();
    bank.transfer(alice, bob, 100);

    assert_eq!(bank.transaction_count(), 1);
}
```

---

### Endianness in Test Values

**When**: Writing tests involving numeric values that get serialized/deserialized.

**Rule**:
- DO: Use values greater than 256 when testing numeric serialization
- DO: Use asymmetric byte patterns (e.g., `0x1234` not `0x1111`)

**Why**: Endianness bugs only manifest when bytes differ. Values ≤ 255 fit in one byte and won't catch byte-order issues.

**Example**:

```rust
// BAD: Value fits in single byte, won't catch endianness bugs
let amount = 42u32;

// BAD: Symmetric pattern, swapped bytes look the same
let amount = 0x1111u32;

// GOOD: Multi-byte asymmetric value exposes endianness issues
let amount = 0x12345678u32;
// If endianness is wrong, you'll see 0x78563412 instead
```

---

### Tempdir Lifetime in Tests

**When**: Using `tempfile::TempDir` in tests with async code or spawned threads.

**Rule**:
- DO: Declare `TempDir` at the very top of the test function
- DON'T: Create it inside a block or after other setup that might use it

**Why**: `TempDir` is deleted when dropped. If it's dropped before async tasks or threads finish, they'll get "file not found" errors.

**Example**:

```rust
// BAD: tempdir may be dropped before spawned task completes
#[tokio::test]
async fn test_async_write() {
    let handle = {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.txt");
        tokio::spawn(async move {
            tokio::fs::write(&path, b"hello").await // May fail: dir already dropped!
        })
    };
    handle.await.unwrap();
}

// GOOD: tempdir declared at top, lives for entire test
#[tokio::test]
async fn test_async_write() {
    let dir = tempfile::tempdir().unwrap();  // Lives until end of test
    let path = dir.path().join("data.txt");

    let handle = tokio::spawn(async move {
        tokio::fs::write(&path, b"hello").await
    });
    handle.await.unwrap().unwrap();
}
```

---

## Documentation

### Using `#[allow(missing_docs)]`

**When**: A struct field or item has no meaningful documentation beyond its name.

**Rule**:
- DON'T: Add redundant documentation that just restates the name
- DO: Add `#[allow(missing_docs)]` inline when documentation would be noise
- DO: Prefer writing meaningful documentation when possible

**Why**: Redundant docs clutter rendered documentation. But most constructs deserve real documentation even if usage seems obvious.

**Example**:

```rust
// BAD: Redundant documentation that adds no value
pub struct Config {
    /// The name of the config.
    pub name: String,
    /// The id of the config.
    pub id: u64,
}

// GOOD: Suppress lint when truly nothing to add
pub struct Point {
    #[allow(missing_docs)]
    pub x: f64,
    #[allow(missing_docs)]
    pub y: f64,
}

// BEST: Actually document the meaning/constraints
pub struct Config {
    /// Human-readable identifier, must be unique within a namespace.
    /// Used in logs and error messages.
    pub name: String,
    /// Monotonically increasing ID assigned at creation time.
    /// Guaranteed unique across all configs.
    pub id: u64,
}
```
