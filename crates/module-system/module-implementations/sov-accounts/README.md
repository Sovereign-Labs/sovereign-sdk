# `sov-accounts` module

The `sov-accounts` module resolves transaction credentials to rollup
addresses and records which credentials may act for which addresses.

### The `sov-accounts` module offers the following functionality

1. A credential has a deterministic canonical address, computed as
   `credential_id.into::<S::Address>()`. This relation is stateless: using the
   canonical address does not require an account entry to be written.

1. A credential can be explicitly mapped to a primary address in module state.
   This is used for custom account mappings, duplicate credential checks,
   credential-only resolution, and the `get_account` query.

1. It is possible to register another credential for the caller's address using
   the `CallMessage::InsertCredentialId(..)` message. The call fails if that
   credential is already explicitly mapped to any address.

1. It is possible to query the `sov-accounts` module using the `get_account`
   method and get the explicitly mapped account corresponding to the given
   credential id.

## Credential and Address Relations

The module has three credential/address relations. They answer different
questions and should not be treated as interchangeable.

### Stateless canonical address

```text
credential_id -> credential_id.into::<S::Address>()
```

This is the default address for a credential. It is deterministic and requires
no state write. If a credential has no explicit state mapping, this canonical
address is the natural fallback for credential-only routing.

### Credential-to-account map

```text
accounts[credential_id] = Account { addr }
```

This state map records the primary address explicitly associated with a
credential. A credential has at most one primary address in this map, while one
address may be the primary address for many credentials.

This map is used when callers only know a `CredentialId` and need an address:
`resolve_sender_address`, `get_account`, duplicate credential checks, and
legacy/custom account mappings all depend on this credential-indexed lookup.
`get_account` reports entries from this explicit map; it does not mean that
every possible stateless canonical address has a stored account entry.

### Account-credential authorization map

```text
account_owners[(address, credential_id)] = true
```

This state map records authorization. A present entry means the credential is
authorized to sign transactions that execute as the given address.

This relation answers "may this credential act as this address?" once the target
address is known. It is not a replacement for the credential-to-account map,
because it is keyed by `(address, credential_id)` and cannot efficiently answer
"which address is this credential registered to?" with a single point lookup.

In short: routing from only a credential uses the explicit
credential-to-account map or the stateless canonical fallback. Authorization for
a known address uses `account_owners`.
