# `sov-accounts` module

The `sov-accounts` module resolves transaction credentials to rollup
addresses and records which credentials may act for which addresses.

### The `sov-accounts` module offers the following functionality

1. A credential has a deterministic canonical address, computed as `credential_id.into::<S::Address>()`. 
   This relation is stateless: using the canonical address does not require an account entry to be written.

2. Legacy state can contain an explicit `credential_id -> address` mapping.
   This map is used for credential-only resolution and the `get_account` query.

3. It is possible to authorize another credential for the caller's address using
   the `CallMessage::InsertCredentialId(..)` message. 
   This writes an `account_owners` authorization, not a credential-indexed account entry.

4. It is possible to query the `sov-accounts` module using the `get_account`
   method and get the legacy/custom mapped address corresponding to the given credential id.

## Credential and Address Relations

The module has three credential/address relations.
They answer different questions and should not be treated as interchangeable.

### Stateless canonical address

```text
credential_id -> credential_id.into::<S::Address>()
```

This is the default address for a credential. It is deterministic and requires no state write.
If a credential has no explicit credential-indexed state mapping,
this canonical address is the natural fallback for credential-only routing.

### Credential-to-account map

```text
accounts[credential_id] = Account { addr }
```

This legacy/custom state map records the primary address explicitly associated with a credential.
A credential has at most one primary address in this map,
while one address may be the primary address for many credentials.

This map is used when callers only know a `CredentialId` and need an address:
`resolve_sender_address`, `get_account`, and legacy/custom account mappings all depend on this credential-indexed lookup. 
New `InsertCredentialId` calls do not write this map.
`get_account` reports entries from this explicit map; 
it does not mean that every possible stateless canonical address or explicit authorization has a stored account entry.

### Account-credential authorization map

```text
account_owners[(address, credential_id)] = true
```

This state map records authorization. 
A present entry means the credential is authorized to sign transactions that execute as the given address.
The key is the exact `(address, credential_id)` pair, so this relation does not provide a credential-only lookup by itself.

This relation answers "may this credential act as this address?" once the target address is known.
It is not a replacement for the credential-to-account map,
because it is keyed by `(address, credential_id)` and cannot efficiently answer
"which address is this credential mapped to?" with a single point lookup.
New `InsertCredentialId` calls write this relation.

In short: routing from only a credential uses the explicit credential-to-account
map or the stateless canonical fallback. 
Callers that need to verify whether a known address may be used with a credential should use the combined
`is_authorized_for` check: legacy/custom mapping, stateless canonical address, or `account_owners`.
