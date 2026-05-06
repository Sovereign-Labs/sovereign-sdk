# `sov-accounts` module

The `sov-accounts` module resolves transaction credentials to rollup
addresses and records which credentials may act for which addresses.

### The `sov-accounts` module offers the following functionality

1. A credential has a deterministic canonical address, computed as `credential_id.into::<S::Address>()`.
   This relation is stateless: using the canonical address does not require an account entry to be written.

2. It is possible to authorize another credential for the caller's address using
   the `CallMessage::InsertCredentialId(..)` message.
   This writes an `account_owners` authorization.

## Credential and Address Relations

### Stateless canonical address

```text
credential_id -> credential_id.into::<S::Address>()
```

This is the default address for a credential. It is deterministic and requires no state write.
If a credential has no explicit authorization, this canonical address is the natural fallback.

### Account-credential authorization map

```text
account_owners[(address, credential_id)] = true
```

This state map records authorization.
A present entry means the credential is authorized to sign transactions that execute as the given address.
The key is the exact `(address, credential_id)` pair, so this relation does not provide a credential-only lookup by itself.

This relation answers "may this credential act as this address?" once the target address is known.
New `InsertCredentialId` calls write this relation.

Callers that need to verify whether a known address may be used with a credential should use
`is_authorized_for`, which checks the stateless canonical address and `account_owners`.

## Upgrade procedure for chains with legacy `accounts` entries

The module used to have a separate `accounts` mapping with different semantics, now deprecated and unused.
Chains whose genesis was before the `accounts` deprecation need to have a migration run at the upgrade height
(including whenever resyncing from genesis).

The migration ships as a CLI binary in `examples/demo-rollup`:

```sh
# Inspect what would change without committing.
cargo run --features migration-script \
    --bin legacy-accounts-migrate -- \
    --rollup-config-path /path/to/rollup_config.toml \
    --dry-run

# Commit the migration in-place at the current head version.
cargo run --features migration-script \
    --bin legacy-accounts-migrate -- \
    --rollup-config-path /path/to/rollup_config.toml
```

The binary requires a stopped node (the storage manager opens the DB exclusively).
The reported `pre_state_root` and `post_state_root` are written in JSON; verify the
post-root matches what the rollup loads on restart.

### Behavior change: canonical-address authorization is no longer suppressed

A pre-migration `accounts[C] = A` row was authoritative: `is_authorized_for(X, C)` returned strictly
`A == X` and bypassed the canonical-address fallback. The refactored `is_authorized_for` returns
`true` whenever `X == credential_id.into::<S::Address>()` or `account_owners[(X, C)] = true`, with
no suppression.

The migration converts each `accounts[C] = A` row into `account_owners[(A, C)] = true`. For any
legacy row where `A != canonical(credential_id)`, the credential gains authorization to act as
`canonical(credential_id)` after the migration, in addition to keeping authorization for `A`. The
canonical fallback is computed, not stored, so it cannot be revoked through `account_owners`.

Operators should treat each migrated entry as also implicitly authorizing
`credential_id.into::<S::Address>()`. If that address holds assets or permissions whose security
relied on the legacy exclusive semantic, retire the credential before deploying the new binary.

The migration requires NOMT prefix iteration; JMT-backed deployments are not supported.

For non-demo rollups, copy `examples/demo-rollup/src/migrations/legacy_accounts.rs`
and swap in your own runtime/spec types — the migration logic itself lives in
`sov_accounts::migrations` and is reusable.
