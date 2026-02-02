# Blacklist Module

A module that manages a blacklist of wallet addresses.

## Overview

- Wallets can be added to or removed from the blacklist
- Enforcement: `enforce_not_blacklisted` fails if wallet is blacklisted

## State

- `owner`: Address with ultimate control
- `manager`: Operational address for day-to-day configuration
- `enforcement_enabled`: Global toggle for enforcement
- `blacklisted`: Map of wallet addresses to blacklist status
- `blacklist_signers`: Addresses authorized to modify the blacklist

## Call Messages

- `SetManager`: Change the manager address (owner-only)
- `SetEnforcementEnabled`: Toggle enforcement (owner-only)
- `SetBlacklistSigner`: Grant/revoke signer privileges (manager-only)
- `SetBlacklisted`: Add/remove a wallet from blacklist (signer-only)
- `SetBlacklistedBatch`: Batch add/remove wallets (signer-only)
- `EnforceNotBlacklisted`: Assert wallet is not blacklisted

## Public API

- `is_blacklisted(wallet)` - Check if wallet is blacklisted
- `enforce_not_blacklisted(wallet)` - Fail if wallet is blacklisted
