# Implementation Plan: Sequencing Data in Transactions

## Overview
Add support for a new transaction type (Version2) with `sequencing_data: Vec<u8>` field that is **not signed by the user** but filled by the sequencer. This enables use cases like high-precision timestamps that exceed the timestamp oracle's capabilities.

## Requirements
1. New `Version2` transaction type with `sequencing_data: Vec<u8>` field
2. Extend `Context<S>` to provide access to sequencing data
3. Sequencer fills the field before transaction execution
4. Field must NOT be included in signature verification
5. Field MUST be included in transaction hash (for uniqueness/replay protection)

---

## Architecture Decision: Extend Context (Option A)

**Why extend Context instead of creating a new capability trait:**

✅ **Modules already receive `Context` in their `call()` method**
✅ **Context already provides sequencer addresses**
✅ **Modules already have state access via the `state` parameter**
✅ **Minimal API surface - no new traits or parameters**
✅ **Simple to implement and use**

```rust
// Module already has this signature:
fn call(
    &mut self,
    message: Self::CallMessage,
    context: &Context<Self::Spec>,      // ← Already here!
    state: &mut impl TxState<Self::Spec>, // ← Already have state!
) -> Result<(), Self::Error>;
```

---

## Implementation Steps

### Step 1: Add Version2 Transaction Type

**File**: `crates/module-system/sov-modules-api/src/transaction/types/v2.rs` (NEW FILE)

**Create the new version**:
```rust
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_universal_wallet::schema::UniversalWallet;

use crate::capabilities::UniquenessData;
use crate::transaction::{TxDetails, hex_field_format};
use crate::{CryptoSpecExt, Spec};

/// V2 transaction with sequencer-provided metadata.
///
/// This transaction type includes a `sequencing_data` field that is filled by the sequencer
/// and is NOT signed by the user. This allows the sequencer to add metadata like high-precision
/// timestamps that are not available to users at transaction creation time.
///
/// # Security Note
/// The `sequencing_data` field is:
/// - NOT included in signature verification (user doesn't sign it)
/// - IS included in the transaction hash (for uniqueness/replay protection)
/// - Fully controlled by the sequencer (users cannot forge it)
#[derive(
    derive_more::Debug,
    Clone,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[serde(bound = "Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct Version2<Call, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
    /// The signature of the transaction.
    #[serde(with = "hex_field_format")]
    #[sov_wallet(display = "hex")]
    pub signature: C::Signature,

    /// The public key of the sender of the transaction.
    #[serde(with = "hex_field_format")]
    #[sov_wallet(display = "hex")]
    pub pub_key: C::PublicKey,

    /// The runtime call of the transaction.
    #[sov_wallet(
        bound = "Call: sov_rollup_interface::sov_universal_wallet::schema::UniversalWallet"
    )]
    pub runtime_call: Call,

    /// Uniqueness identifier of this transaction.
    pub uniqueness: UniquenessData,

    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,

    /// Sequencing data provided by the sequencer (NOT signed by user).
    /// This field is filled by the sequencer after receiving the transaction.
    /// Common uses: high-precision timestamps, sequencer metadata, etc.
    pub sequencing_data: Vec<u8>,
}

#[cfg(feature = "native")]
impl<Call: BorshSerialize, S: Spec, C: CryptoSpecExt> Version2<Call, S, C> {
    /// Signs the transaction with the given key but does not add the signature.
    pub fn sign_without_adding(&self, key: &C::PrivateKey, chain_hash: &[u8; 32]) -> C::Signature {
        key.sign(&self.serialize_for_signing(chain_hash))
    }

    /// Serializes the transaction for signing - EXCLUDES sequencing_data.
    /// This is critical: the user signs the transaction before sequencing_data is added.
    pub fn serialize_for_signing(&self, chain_hash: &[u8; 32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        BorshSerialize::serialize(&self.runtime_call, &mut out)
            .expect("Serialization to vec is infallible");
        BorshSerialize::serialize(&self.uniqueness, &mut out)
            .expect("Serialization to vec is infallible");
        BorshSerialize::serialize(&self.details, &mut out)
            .expect("Serialization to vec is infallible");
        // NOTE: sequencing_data is NOT included in signature
        out.extend_from_slice(chain_hash);
        out
    }
}

impl<Call, S: Spec, C: CryptoSpecExt> Version2<Call, S, C> {
    /// Returns the sequencing data.
    #[must_use]
    pub fn sequencing_data(&self) -> &[u8] {
        &self.sequencing_data
    }

    /// Sets the sequencing data. Should only be called by the sequencer.
    pub fn set_sequencing_data(&mut self, data: Vec<u8>) {
        self.sequencing_data = data;
    }
}
```

**Update mod.rs**:
```rust
// In crates/module-system/sov-modules-api/src/transaction/types/mod.rs
pub mod v0;
pub mod v1;
pub mod v2;  // NEW
```

---

### Step 2: Add V2 to Transaction Enum

**File**: `crates/module-system/sov-modules-api/src/transaction/mod.rs`

**Add V2 variant**:
```rust
pub use types::{v0::Version0, v1::Version1, v2::Version2};

pub enum Transaction<R: TransactionCallable, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
    /// V0 Transaction type.
    V0(Version0<R::Call, S, C>),
    /// V1 (multisig) transaction.
    V1(Version1<R::Call, S, C>),
    /// V2 transaction with sequencing data.
    V2(
        #[borsh(bound(
            serialize = "<C as CryptoSpec>::Signature: BorshSerialize, <C as CryptoSpec>::PublicKey: BorshSerialize",
            deserialize = "<C as CryptoSpec>::Signature: BorshDeserialize, <C as CryptoSpec>::PublicKey: BorshDeserialize",
        ))]
        Version2<R::Call, S, C>,
    ),
}

impl<R: TransactionCallable, S: Spec> From<Version2<R::Call, S>> for Transaction<R, S> {
    fn from(value: Version2<R::Call, S>) -> Self {
        Transaction::V2(value)
    }
}
```

**Update all match statements** (8 places):
```rust
pub fn runtime_call(&self) -> &R::Call {
    match &self {
        Transaction::V0(inner) => &inner.runtime_call,
        Transaction::V1(inner) => &inner.runtime_call,
        Transaction::V2(inner) => &inner.runtime_call,  // NEW
    }
}

pub fn chain_id(&self) -> u64 {
    match &self {
        Transaction::V0(inner) => inner.details.chain_id,
        Transaction::V1(inner) => inner.details.chain_id,
        Transaction::V2(inner) => inner.details.chain_id,  // NEW
    }
}

// ... similarly for call(), to_unsigned_transaction(), charge_gas_for_signature(),
// verify_signature_unmetered() ...
```

**Add helper method for sequencing data**:
```rust
impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Transaction<R, S, C> {
    /// Returns the sequencing data if this is a V2 transaction, None otherwise.
    #[must_use]
    pub fn sequencing_data(&self) -> Option<&[u8]> {
        match self {
            Transaction::V0(_) => None,
            Transaction::V1(_) => None,
            Transaction::V2(inner) => Some(&inner.sequencing_data),
        }
    }
}
```

---

### Step 3: Extend Context to Include Sequencing Data

**File**: `crates/module-system/sov-modules-api/src/module/spec.rs`

**Add field to Context**:
```rust
#[derive(Clone, Debug)]
pub struct Context<S: Spec> {
    /// The original credentials of the sender.
    sender_credentials: Credentials,
    /// The sender address of the transaction.
    sender: S::Address,
    /// The rollup address of the sequencer who included the transaction.
    sequencer: S::Address,
    /// The DA layer address of the sequencer who included the transaction.
    sequencer_da_address: <S::Da as DaSpec>::Address,
    /// Sequencing data provided by the sequencer (V2 transactions only).
    sequencing_data: Option<Vec<u8>>,  // NEW
}

impl<S: Spec> Context<S> {
    /// Creates a new execution context.
    pub fn new(
        sender: S::Address,
        sender_credentials: Credentials,
        sequencer: S::Address,
        sequencer_da_address: <S::Da as DaSpec>::Address,
    ) -> Self {
        Self {
            sender,
            sender_credentials,
            sequencer,
            sequencer_da_address,
            sequencing_data: None,
        }
    }

    /// Creates a new execution context with sequencing data.
    pub fn new_with_sequencing_data(
        sender: S::Address,
        sender_credentials: Credentials,
        sequencer: S::Address,
        sequencer_da_address: <S::Da as DaSpec>::Address,
        sequencing_data: Option<Vec<u8>>,
    ) -> Self {
        Self {
            sender,
            sender_credentials,
            sequencer,
            sequencer_da_address,
            sequencing_data,
        }
    }

    /// Returns the sequencing data for V2 transactions, None for V0/V1.
    #[must_use]
    pub fn sequencing_data(&self) -> Option<&[u8]> {
        self.sequencing_data.as_deref()
    }
}
```

---

### Step 4: Update STF Blueprint to Pass Sequencing Data

**File**: `crates/module-system/sov-modules-stf-blueprint/src/sequencer_mode/common.rs`

**Modify `apply_tx` to extract and pass sequencing data**:
```rust
pub fn apply_tx<S, RT, I>(
    runtime: &mut RT,
    ctx: &Context<S>,  // Context now contains sequencing_data
    tx: &AuthenticatedTransactionData<S>,
    raw_tx_hash: TxHash,
    raw_tx: FullyBakedTx,
    message: <RT as DispatchCall>::Decodable,
    mut working_set: WorkingSet<S, I>,
) -> (ApplyTxResult<S>, TxScratchpad<S, I>)
// ... rest unchanged, Context already has the data
```

**File**: `crates/module-system/sov-modules-stf-blueprint/src/sequencer_mode/registered.rs`

**Update where Context is created** (search for `Context::new`):
```rust
// OLD:
let context = Context::new(
    sender,
    auth_data.credentials.clone(),
    sequencer_rollup_address,
    sequencer.clone(),
);

// NEW:
let sequencing_data = transaction.sequencing_data().map(|d| d.to_vec());
let context = Context::new_with_sequencing_data(
    sender,
    auth_data.credentials.clone(),
    sequencer_rollup_address,
    sequencer.clone(),
    sequencing_data,
);
```

---

### Step 5: Sequencer Fills Sequencing Data

**File**: `crates/full-node/sov-sequencer/src/preferred/block_executor.rs` (or similar)

**Before storing/executing a V2 transaction**:
```rust
// Example: Fill with high-precision timestamp
if let Transaction::V2(ref mut v2_tx) = tx {
    let timestamp_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    // Encode as little-endian bytes
    v2_tx.set_sequencing_data(timestamp_nanos.to_le_bytes().to_vec());
}

// Now compute hash and store - sequencing_data is included in hash
```

**Important**: This must happen:
1. AFTER receiving the transaction from the user
2. BEFORE computing the final transaction hash
3. BEFORE storing to database/DA layer

---

### Step 6: Usage Example (Mirage's Use Case)

**Example module implementation**:
```rust
use sov_modules_api::Module;

pub struct TimingModule<S: Spec> {
    timestamps: sov_state::StateMap<u128, TimestampRecord>,
    // ...
}

impl<S: Spec> Module for TimingModule<S> {
    type Spec = S;
    // ... other associated types ...

    fn call(
        &mut self,
        message: Self::CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        // Access sequencing data from context
        if let Some(data) = context.sequencing_data() {
            // Parse high-precision timestamp
            if data.len() == 16 {
                let timestamp_nanos = u128::from_le_bytes(
                    data.try_into().expect("Length already checked")
                );

                // Use sequencer info (already in Context)
                let sequencer = context.sequencer();

                // Store to state (already have access)
                self.timestamps.set(
                    &timestamp_nanos,
                    &TimestampRecord {
                        sequencer: sequencer.clone(),
                        timestamp_nanos,
                        data: message.data,
                    },
                    state,
                );
            }
        }

        // Continue with normal call processing
        match message {
            // ... handle messages ...
        }

        Ok(())
    }
}
```

---

## Security Considerations

### 1. Signature Verification
- ✅ `sequencing_data` is **excluded** from `serialize_for_signing()`
- ✅ User signs: `serialize(runtime_call, uniqueness, details) + chain_hash`
- ✅ Sequencer can modify `sequencing_data` without invalidating signature
- ⚠️ Test thoroughly: malicious modification must not break security

### 2. Transaction Hash
- ✅ `sequencing_data` is **included** in `borsh::to_vec(&transaction)`
- ✅ Different sequencing_data = different hash
- ✅ Prevents replay: same tx with different timestamps = different hashes
- ✅ Uniqueness module sees the full hash including sequencing_data

### 3. Trust Model
- ⚠️ **Sequencing data is fully trusted** - sequencer can put anything
- ✅ Users cannot forge or modify it
- ✅ Suitable for: timestamps, block height, sequencer metadata
- ❌ NOT suitable for: user-controlled data, authentication

### 4. Size Limits
**Recommendation**: Enforce maximum size to prevent DoS:
```rust
const MAX_SEQUENCING_DATA_SIZE: usize = 1024; // 1KB

impl<Call, S: Spec, C: CryptoSpecExt> Version2<Call, S, C> {
    pub fn set_sequencing_data(&mut self, data: Vec<u8>) -> Result<(), Error> {
        if data.len() > MAX_SEQUENCING_DATA_SIZE {
            return Err(Error::SequencingDataTooLarge);
        }
        self.sequencing_data = data;
        Ok(())
    }
}
```

### 5. Gas Accounting
**Question**: Should sequencing_data count toward gas?
- Users don't control it, so probably not
- But storage costs should be accounted for
- **Recommendation**: Don't charge users, but limit size

---

## Testing Strategy

### Test Cases

**File**: `crates/module-system/sov-modules-api/src/transaction/tests.rs`

```rust
#[test]
fn test_v2_signature_excludes_sequencing_data() {
    // Create V2 tx with empty sequencing_data
    let mut tx = create_v2_transaction_empty_sequencing_data();

    // Sign it
    let signature = tx.sign_without_adding(&priv_key, &chain_hash);

    // Modify sequencing_data
    tx.set_sequencing_data(vec![1, 2, 3, 4]);

    // Signature should still be valid!
    assert!(tx.verify_signature_unmetered(&msg).is_ok());
}

#[test]
fn test_v2_hash_includes_sequencing_data() {
    let mut tx1 = create_v2_transaction();
    tx1.set_sequencing_data(vec![1, 2, 3]);

    let mut tx2 = tx1.clone();
    tx2.set_sequencing_data(vec![4, 5, 6]);

    // Different sequencing_data = different hash
    assert_ne!(tx1.hash(), tx2.hash());
}

#[test]
fn test_v0_v1_still_work() {
    // Ensure backwards compatibility
    let v0 = create_v0_transaction();
    assert!(v0.verify_signature_unmetered(&msg).is_ok());

    let v1 = create_v1_transaction();
    assert!(v1.verify_signature_unmetered(&msg).is_ok());
}

#[test]
fn test_context_provides_sequencing_data() {
    let sequencing_data = Some(vec![1, 2, 3, 4]);
    let ctx = Context::new_with_sequencing_data(
        sender, creds, sequencer, sequencer_da, sequencing_data,
    );

    assert_eq!(ctx.sequencing_data(), Some(&[1, 2, 3, 4][..]));
}
```

**Integration Test**: `crates/module-system/integration-tests/tests/sequencing_data.rs`

```rust
#[test]
fn test_module_receives_sequencing_data() {
    // Create V2 transaction
    // Fill sequencing_data in sequencer
    // Execute transaction
    // Verify module received the data via Context
}
```

---

## Migration Path

### Phase 1: Infrastructure (No Breaking Changes) ✅
- [x] Add `Version2` transaction type
- [x] Add `V2` variant to `Transaction` enum
- [x] Update all match statements to handle V2
- [x] Extend `Context` with `sequencing_data` field
- [x] Update `Context::new()` to set `sequencing_data: None`
- [x] All existing V0/V1 transactions continue working unchanged

### Phase 2: Sequencer Support
- [ ] Update sequencer to recognize V2 transactions
- [ ] Add timestamp filling logic before storing
- [ ] Ensure proper serialization/deserialization
- [ ] Update database schema if needed

### Phase 3: Customer Integration
- [ ] Document API for Mirage
- [ ] Provide example module implementation
- [ ] Add integration tests
- [ ] Deploy to testnet

---

## Files to Modify

### Core Transaction Types
- ✅ `crates/module-system/sov-modules-api/src/transaction/types/v2.rs` (NEW)
- ✅ `crates/module-system/sov-modules-api/src/transaction/types/mod.rs`
- ✅ `crates/module-system/sov-modules-api/src/transaction/mod.rs`
- ✅ `crates/module-system/sov-modules-api/src/transaction/unsigned.rs` (update for V2)

### Context Extension
- ✅ `crates/module-system/sov-modules-api/src/module/spec.rs`

### STF Blueprint
- ✅ `crates/module-system/sov-modules-stf-blueprint/src/sequencer_mode/common.rs`
- ✅ `crates/module-system/sov-modules-stf-blueprint/src/sequencer_mode/registered.rs`
- ✅ `crates/module-system/sov-modules-stf-blueprint/src/sequencer_mode/unregistered.rs`

### Sequencer
- ⏳ `crates/full-node/sov-sequencer/src/preferred/block_executor.rs`
- ⏳ `crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/inner.rs`

### Tests
- ⏳ `crates/module-system/sov-modules-api/src/transaction/tests.rs`
- ⏳ `crates/module-system/integration-tests/tests/sequencing_data.rs` (NEW)

---

## Summary

**Minimal, focused approach:**
1. Add `Version2` transaction type with `sequencing_data: Vec<u8>`
2. Extend `Context<S>` to include optional sequencing data
3. Sequencer fills the field before execution
4. Modules access via `context.sequencing_data()`

**Key advantages:**
- ✅ Simple API - no new traits
- ✅ Modules already have everything (Context + state access)
- ✅ Backwards compatible (V0/V1 unchanged)
- ✅ Secure (sequencing_data excluded from signature, included in hash)
- ✅ Flexible (raw bytes, each use case parses as needed)

**Next step**: Start with Phase 1 - add the transaction type infrastructure without changing any execution logic yet.
