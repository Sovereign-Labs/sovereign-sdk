# `sb-session-registry`

A module that maintains per-wallet “trading sessions” for a single application on a rollup.

Used when other runtime modules (e.g., a DEX) need to enforce that a wallet has a **present** session and/or an **active** (non-expired) session.

## How It Works

The module stores session state keyed by wallet address:

- **Session record:** `{ expiry_ts, bypass }`
- **Present session:** `bypass == true` OR `expiry_ts != 0`
- **Active session:** `bypass == true` OR `(expiry_ts + expiry_offset) > now`
- **Deletion:** setting `expires_at == 0` removes the session entry

## Roles and Access Control

Access control is enforced in `call::execute` based on `context.sender()`:

- **Owner**
  - `SetManager`
  - `SetEnforcementEnabled`
  - `SetExpiryOffset`
- **Manager**
  - `SetSessionSigner`
  - `SetBypass`
- **Session Signer**
  - `SetSession`
  - `SetSessionBatch`
- **Anyone**
  - `EnforceSessionActive`
  - `EnforceSessionPresent`

## Integration Guide

### Step 1: Add a reference to your module

In any module that wants to enforce sessions, add a field of type `sb_session_registry::SessionRegistry<S>`.

```rust, ignore
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct YourApp<S: Spec> {
    #[id]
    pub id: ModuleId,

    // ... other fields

    #[module]
    pub session_registry: sb_session_registry::SessionRegistry<S>,
}
```

### Step 2: Enforce session requirements in your logic

Typical patterns:

```rust, ignore
// Require an *active* session (bypass OR not expired)
self.session_registry.enforce_session_active(&wallet, state)?;

// Require a *present* session (exists and not deleted)
self.session_registry.enforce_session_present(&wallet, state)?;
```

If you want a boolean check (without error):

```rust, ignore
let is_active = self.session_registry.is_session_active(&wallet, state)?;
let is_present = self.session_registry.is_session_present(&wallet, state)?;
```

## Runtime Administration

| Message                                   | Purpose                                      | Notes                                                                                    |
| ----------------------------------------- | -------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `SetManager { new_manager }`              | Update manager address                       | Owner-only; emits `ManagerSet { old_manager, new_manager }`                              |
| `SetEnforcementEnabled { enabled }`       | Toggle global enforcement                    | Owner-only; emits `EnforcementEnabledSet { enabled }`                                    |
| `SetSessionSigner { signer, allowed }`    | Grant/revoke session-signer privileges       | Manager-only; emits `SessionSignerSet { signer, allowed }`                               |
| `SetSession { wallet, expires_at }`       | Set or delete a single session               | Session-signer-only; `expires_at == 0` deletes; emits `SessionSet { wallet, expiry_ts }` |
| `SetSessionBatch { wallets, expiries }` | Set or delete sessions for a batch           | Session-signer-only                                                                      |
| `SetBypass { wallet, bypass }`            | Set/clear per-wallet bypass                  | Manager-only; emits `BypassSet { wallet, bypass }`                                       |
| `SetExpiryOffset { new_offset }`          | Set global expiry extension                  | Owner-only; emits `ExpiryOffsetUpdated { old_offset, new_offset }`                       |
| `EnforceSessionActive { wallet }`         | Transaction endpoint: assert active session  | Respects `enforcement_enabled`                                                           |
| `EnforceSessionPresent { wallet }`        | Transaction endpoint: assert present session | Respects `enforcement_enabled`                                                           |
