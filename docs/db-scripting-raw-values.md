# Offline DB Scripting With Raw Container Values

This guide describes native-only APIs for migration scripts that keep typed keys but read/write raw value bytes (`Vec<u8>`).

## Scope

- Intended for offline scripts against DB state.
- Prefix iteration support is storage-dependent:
  - NOMT: user and kernel prefix iteration can be available.
  - JMT: prefix iteration returns `None`.
- Accessory prefix scans are not currently exposed; use key-driven fallback iteration.

## Raw APIs

All methods below are gated behind `feature = "native"`.

- `StateValue`:
  - `get_raw`
  - `set_raw`
  - `remove_raw`
- `StateMap`:
  - `get_raw`
  - `set_raw`
  - `remove_raw`
  - `iter_raw` (user/kernel namespaces, storage-dependent)
  - `iter_raw_from_keys` (all namespaces, backend-agnostic fallback)
- `StateVec`:
  - `get_raw`
  - `set_raw`
  - `push_raw`
  - `pop_raw`
  - `iter_raw`

`StateVec` length is still read as typed `u64`; only element payloads are treated as raw bytes.

## Migration Pattern

```rust
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{KernelStateMap, StateCheckpoint};
use sov_state::codec::BorshCodec;
use sov_state::Prefix;

#[derive(BorshDeserialize)]
struct OldValue {
    amount: u64,
}

#[derive(BorshSerialize)]
struct NewValue {
    amount: u128,
}

fn migrate<S: sov_modules_api::Spec>(
    map: &mut KernelStateMap<u64, NewValue>,
    state: &mut StateCheckpoint<S>,
    keys: Vec<u64>,
) -> anyhow::Result<()> {
    for (key, maybe_raw) in map.iter_raw_from_keys(keys, state)? {
        let Some(raw) = maybe_raw else {
            continue;
        };

        let old = OldValue::try_from_slice(&raw)?;
        let new = NewValue {
            amount: old.amount as u128,
        };
        let new_raw = borsh::to_vec(&new)?;
        map.set_raw(&key, &new_raw, state)?;
    }

    Ok(())
}
```

## Iteration Strategy

1. Prefer `StateMap::iter_raw` for user/kernel namespace migrations when storage returns an iterator.
2. If `iter_raw` returns `None`, use `iter_raw_from_keys` with a key list from your script input.
3. For accessory migrations, use `iter_raw_from_keys`.
