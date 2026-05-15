# `sov-accounts` module

The `sov-accounts` module resolves transaction credentials to rollup
addresses and records which credentials may act for which addresses.

### The `sov-accounts` module offers the following functionality

1. A credential has a deterministic canonical address, computed as `credential_id.into::<S::Address>()`.
   This relation is stateless: using the canonical address does not require an account entry to be written.

2. It is possible to authorize another credential for the caller's address using
   the `CallMessage::InsertCredentialId(..)` message.
   This writes an `account_owners` authorization.

3. It is possible to explicitly authorize a credential for the caller's own
   address with `CallMessage::AddCredentialToAddress { address, credential }`,
   revoke such an authorization with
   `CallMessage::RemoveCredentialFromAddress { address, credential }`, and
   atomically swap one credential for another with
   `CallMessage::RotateCredentialOnAddress { address, old_credential, new_credential }`.
   All three calls require `message.address == context.sender()`: callers can
   only modify credentials on the address they are currently signing as. The
   V1 signing path's `target_address` field lets a caller signing with a
   credential authorized for multiple addresses select which one to act as.
   There is no orphan guard on remove — revoking the last credential leaves
   the address unspendable via `account_owners`.

4. It is possible to create a new *synthetic* address — an address whose
   authorization lives purely in `account_owners` and which has no
   naturally-corresponding private key — with
   `CallMessage::CreateSyntheticAddress { salt }`. The address is derived
   by hashing `(domain || visible_slot_hash || sender_addr ||
   sender_credential || salt)` with `S::CryptoSpec::Hasher`, converting
   the resulting 32 bytes to `CredentialId`, and then to `S::Address`.
   The caller's current credential is auto-authorized for it. Different
   callers, salts, and visible slots produce different addresses;
   replaying the same tuple in the same slot is an idempotent no-op.

   `visible_slot_hash` is part of the derivation by design: an attacker
   who later compromises the caller's private key cannot reconstruct
   the same synthetic address off-chain, because the slot hash only
   becomes known once chain progress commits to it and is unforgeable
   without participating in consensus. This makes the call
   *bind-then-use*, not *counterfactual*: unlike CREATE2 or ERC-4337,
   the address cannot be predicted and prefunded ahead of the
   `CreateSyntheticAddress` transaction. Callers must wait for
   finalization and read the derived address from the emitted
   `SyntheticAddressCreated` event before routing assets or permissions
   to it.

   The resulting synthetic address is operated on through the same
   `AddCredentialToAddress` / `RemoveCredentialFromAddress` /
   `RotateCredentialOnAddress` calls as any other address.


## Credential and Address Relations

### Stateless canonical address

```text
canonical(credential_id) = credential_id.into::<S::Address>()
```

This is a deterministic, stateless derivation. It requires no state write. For
specs whose `S::Address` is a simple hash-derived address, an unauthorized
credential can still act as `canonical(credential_id)` thanks to the canonical
fallback in `is_authorized_for`. For composite address specs (e.g.
`MultiAddress<VmAddress>`), `canonical(credential_id)` is only one of several
possible "natural" addresses for a credential — see the worked example below.

### Authenticator-declared default address

```text
default_address = authenticator(credential_id)
```

This is the address the authenticator chooses to admit the transaction as,
absent an explicit `address_override`. It is **not stateless** and **not
required to equal** `canonical(credential_id)`. The authenticator is trusted
to have verified the credential→default_address binding before producing this
value; the on-chain admit-path treats `default_address` as authoritative
unless `account_owners[(default_address, credential_id)] = false` is
explicitly recorded.

### Account-credential authorization map

```text
account_owners[(address, credential_id)] = true | false
```

This state map records authorization overrides. `true` grants authorization;
`false` explicitly revokes authorization for that pair, including the
stateless canonical fallback. The key is the exact `(address, credential_id)`
pair, so this relation does not provide a credential-only lookup by itself.
New `InsertCredentialId` / `AddCredentialToAddress` calls write `true`;
`RemoveCredentialFromAddress` writes `false`.

### Three predicates

`Accounts` exposes three authorization predicates with deliberately different
fallback semantics. Each answers a different question:

| Function | No-entry fallback | Question answered | Used by |
|---|---|---|---|
| `is_explicitly_authorized(addr, cred)` | `false` | "Is this pair explicitly granted?" | On-chain admit-path when `address_override = Some(_)` |
| `is_default_address_authorized(addr, cred)` | `true` | "Would the chain admit a tx if an authenticator declared this address as default?" | On-chain admit-path when `address_override = None` |
| `is_authorized_for(addr, cred)` | `canonical(cred) == addr` | "Is this credential authorized to act as this address under the canonical-fallback view?" | Internal credential lifecycle checks (revoke, rotate, conflict) |

The REST endpoint `GET /authorizations/{address}/{credential_id}` returns all
three as `admit_as_override`, `admit_as_default`, and `authorized`
(deprecated — kept for backward compatibility).

### Worked example: EVM authenticator divergence

Consider a rollup whose spec is `MultiAddress<EthereumAddress>` and a
transaction signed by EVM `signer = 0xAa...Bb` with `credential_id = C =
keccak256(signer_pubkey)`:

- `canonical(C) = C.into::<S::Address>() = MultiAddress::Standard(...)`
  via the blanket `impl<VmAddress> From<CredentialId> for MultiAddress<VmAddress>`
  in `sov-address/src/lib.rs`.
- `default_address = S::Address::from_vm_address(EthereumAddress(0xAa...Bb)) =
  MultiAddress::Vm(0xAa...Bb)` set by `sov-evm/src/authenticate.rs` in
  `extract_evm_authorization_data`.

The two are different enum discriminants, so `default_address != canonical(C)`
for every EVM-signed transaction. Querying
`GET /authorizations/{default_address}/{C}` with no prior `account_owners`
entry returns:

| Field | Value | Why |
|---|---|---|
| `admit_as_override` | `false` | No explicit entry |
| `admit_as_default` | `true` | No entry → chain trusts authenticator |
| `authorized` (deprecated) | `false` | No entry AND `canonical(C) != default_address` (different `MultiAddress` variants) |

The chain admits these transactions (via the `admit_as_default` path).
Consumers that inspect authorization via the REST endpoint must read
`admit_as_default` — not the deprecated `authorized` field — or they will
incorrectly report EVM signers as unauthorized for addresses the chain
actually authorizes.

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

Worked example: suppose pre-migration `accounts[C] = A` with `A = 0x1111…` and
`canonical(C) = C.into::<S::Address>() = 0x2222…`. Before the migration, `C` could sign only as
`0x1111…`. After the migration, `account_owners[(0x1111…, C)] = true` is written, and the
unsuppressed canonical fallback means `C` can now also sign as `0x2222…` — two addresses, one
credential.

Operators should treat each migrated entry as also implicitly authorizing
`credential_id.into::<S::Address>()`. If that address holds assets or permissions whose security
relied on the legacy exclusive semantic, retire the credential before deploying the new binary.

The migration requires NOMT prefix iteration; JMT-backed deployments are not supported.

For non-demo rollups, copy `examples/demo-rollup/src/migrations/legacy_accounts.rs`
and swap in your own runtime/spec types — the migration logic itself lives in
`sov_accounts::migrations` and is reusable.
