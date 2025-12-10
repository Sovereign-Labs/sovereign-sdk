# Hyperlane Solana Register Module

This module provides a secure way to register Solana users on a Sovereign SDK rollup via Hyperlane's cross-chain messaging protocol. It enables users with Solana wallets to create and link embedded wallets on the rollup, providing a seamless onboarding experience.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Sovereign SDK Module](#sovereign-sdk-module)
- [Solana Program](#solana-program)
- [TypeScript Client](#typescript-client)
- [Security Considerations](#security-considerations)
- [Testing](#testing)

## Overview

The Hyperlane Solana Register system consists of three main components:

1. **Sovereign SDK Module** (`sov-hyperlane-register-module`) - A Rust module that receives and processes registration messages from Solana via Hyperlane
2. **Solana Program** - A Solana program that sends registration messages through the Hyperlane mailbox
3. **TypeScript Client** (`@sovereign-sdk/hyperlane-solana-register`) - A client library for building registration transactions on Solana

## Architecture

### Registration Flow

```
┌──────────────┐         ┌─────────────────┐         ┌──────────────────┐
│ Solana User  │────────▶│ Solana Program  │────────▶│ Hyperlane        │
│              │         │ (Register)      │         │ Mailbox          │
└──────────────┘         └─────────────────┘         └──────────────────┘
                                                              │
                                                              ▼
                         ┌─────────────────┐         ┌──────────────────┐
                         │ Rollup Account  │◀────────│ Sovereign Module │
                         │ Created/Linked  │         │ (Recipient)      │
                         └─────────────────┘         └──────────────────┘
```

### Message Structure

The registration message body contains two 32-byte public keys:
- **Payer Public Key** (bytes 0-31): The Solana wallet initiating the registration
- **Embedded Public Key** (bytes 32-63): The embedded wallet to be linked on the rollup

The embedded public key becomes the `CredentialId` on the rollup, and it's associated with the rollup address derived from the payer's public key.

The result of this process is that the embedded wallet controls the users Solana address on the rollup. This is desirable in situations like Zetas where we want the embedded wallet to be opaque,
with the user feeling like they're using their Solana wallet directly.

---

## Sovereign SDK Module

The `SolanaRegistration<S>` module is a Sovereign SDK module that implements the `Recipient` trait to receive Hyperlane messages.

### Module Location

`src/lib.rs:25-49`

### Key Features

- **Selective Message Handling**: Only processes messages from the configured Solana domain and trusted program ID
- **Account Linking**: Associates Solana embedded wallets with rollup addresses via the `sov-accounts` module
- **Duplicate Prevention**: Rejects attempts to register an embedded wallet that's already linked to a different address
- **Admin Controls**: Allows configuration updates via admin-only calls
- **Fallback to Warp**: Non-Solana messages are forwarded to the underlying `Warp` module

### Configuration

The module requires the following genesis configuration:

```rust
pub struct GenesisConfig<S: Spec> {
    /// The Solana deployment configuration
    pub deployment: Option<SolanaDeployment>,
    /// The Interchain Security Module (ISM) for message verification
    pub ism: Option<Ism>,
    /// The admin address that can update module configuration
    pub admin: S::Address,
}

pub struct SolanaDeployment {
    /// The Solana hyperlane domain id
    pub domain_id: u32,
    /// The trusted program id of the hyperlane-solana-register program
    /// WARNING: This is a TRUSTED program. The owner can arbitrarily
    /// register users which could lead to account takeovers if misused.
    pub program_id: Base58Address,
}
```

We recommend setting the ISM to the same configuration used for warp routes to ensure security and to re-use existing hyperlane validator sets.

### CallMessage API

The module exposes one call message:

```rust
pub enum CallMessage<S: Spec> {
    Update {
        admin: Option<S::Address>,
        deployment: Option<SolanaDeployment>,
        ism: Option<Ism>,
    },
}
```

**Permission**: Only the configured admin can call this message.

**Purpose**: Update the module's configuration including admin address, Solana deployment details, or ISM.

### Events

```rust
pub enum Event<S: Spec> {
    /// Emitted when a user successfully registers
    UserRegistered {
        address: S::Address,
        credential_id: CredentialId,
    },
    /// Emitted when the module configuration is updated
    Updated {
        admin: Option<S::Address>,
        deployment: Option<SolanaDeployment>,
        ism: Option<Ism>,
    },
}
```

### Error Handling

The module defines several error types:

- `AlreadyRegistered`: The embedded public key is already linked to a different address
- `InvalidBodyLength`: The message body doesn't contain exactly 64 bytes
- `ExtractPubKey`: Failed to parse public keys from the message body
- `AdminNotFound`: Admin address not configured
- `Forbidden`: Non-admin attempted to call an admin-only function

### Message Processing Logic

See `src/lib.rs:196-213` for the `handle` implementation:

1. Verify the message origin matches the configured Solana domain ID
2. Verify the sender matches the trusted Solana program ID
3. Extract the two 32-byte public keys from the message body
4. Convert the payer public key to a rollup address
5. Use `sov-accounts` to resolve or create the address-credential mapping
6. Reject if the embedded wallet is already linked to a different address
7. Emit a `UserRegistered` event

---

## Solana Program

The Solana program is a lightweight program that constructs and dispatches Hyperlane messages for user registration.

### Program Location

`solana/program/src/lib.rs`

### Program ID

The default program ID is: `HX6EowhA5XwWj29iTFeqhprg1gUxHgv6RNUu4bRtUgob`

You can configure a custom program ID by updating the `declare_id!` macro in `lib.rs:15`.

### Instruction Format

```rust
pub enum HyperlaneRegisterInstruction {
    SendRegister(RegisterMessage),
}

pub struct RegisterMessage {
    /// The destination domain id (the rollup's Hyperlane domain)
    pub destination: u32,
    /// The pubkey of the embedded user's wallet
    pub embedded_user: Pubkey,
    /// Recipient warp route in hex or base58 encoding
    pub recipient: String,
}
```

### Required Accounts

The instruction requires these accounts in order (see `lib.rs:72-142`):

1. **Mailbox Program** - The trusted Hyperlane mailbox program
2. **Mailbox Outbox PDA** - The outbox account for the mailbox
3. **Dispatch Authority PDA** - Derived from the register program ID
4. **System Program** - Solana's system program
5. **SPL Noop Program** - For logging events
6. **Payer** - The user's wallet (signer, pays for the transaction)
7. **Unique Message Account** - A newly generated keypair (signer)
8. **Dispatched Message PDA** - Derived from the mailbox and unique message account

### Trusted Mailbox Configuration

The program determines the trusted mailbox using the following priority (see `lib.rs:211-225`):

1. **Compile-time**: `HYPERLANE_MAILBOX_PUBKEY` environment variable
2. **Test mode**: `692KZJaoe2KRcD6uhCQDLLXnLNA5ZLnfvdqjE4aX9iu1` (when `test-utils` feature is enabled)
3. **Default**: `75HBBLae3ddeneJVrZeyrDfv6vb7SMC3aCpBucSXS5aR` (Solana testnet)

### Message Body Format

The program constructs the message body as follows (see `lib.rs:166-168`):

```rust
let mut message_body = Vec::with_capacity(64); // 2 x 32 bytes
message_body.extend_from_slice(&payer_info.key.to_bytes());      // bytes 0-31
message_body.extend_from_slice(&register_message.embedded_user.to_bytes()); // bytes 32-63
```

### Deployment

See [solana/docs/DEPLOYMENT.md](solana/docs/DEPLOYMENT.md) for detailed deployment instructions.

**Quick summary**:

```bash
# 1. Install Solana toolchain (version 1.14.20)
./scripts/install-solana-1.14.20.sh

# 2. Build the program
./scripts/build-programs.sh

# 3. Generate a program keypair
solana-keygen new --outfile ./solana/target/deploy/hyperlane_solana_sovereign_register-keypair.json

# 4. Update the declare_id! in solana/program/src/lib.rs with the new program ID

# 5. Deploy to Solana
solana program deploy \
    --program-id ./solana/target/deploy/hyperlane_solana_sovereign_register-keypair.json \
    ./solana/target/deploy/hyperlane_solana_sovereign_register.so
```

### Important Security Notes

- The program MUST be built with Solana SBF compiler version 1.14.20 to match Hyperlane's programs
- Only the configured mailbox program is trusted - the program validates this at `lib.rs:82-89`
- The recipient field in `RegisterMessage` is converted to H256 format for Hyperlane compatibility

---

## TypeScript Client

The TypeScript client library provides a simple API for building registration transactions.

### Installation

```bash
npm install @sovereign-sdk/hyperlane-solana-register
# or
yarn add @sovereign-sdk/hyperlane-solana-register
# or
pnpm add @sovereign-sdk/hyperlane-solana-register
```

### Usage Example

```typescript
import { Connection, Keypair, sendAndConfirmTransaction } from "@solana/web3.js";
import { HyperlaneSolanaRegister } from "@sovereign-sdk/hyperlane-solana-register";

// Initialize the register client
const register = new HyperlaneSolanaRegister({
  mailbox: "75HBBLae3ddeneJVrZeyrDfv6vb7SMC3aCpBucSXS5aR",  // Solana testnet mailbox
  register: "HX6EowhA5XwWj29iTFeqhprg1gUxHgv6RNUu4bRtUgob",  // Register program
  splNoop: "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV",  // Optional, this is the default - keep this unless you know what you're doing
});

// Create or load wallets
const payer = Keypair.fromSecretKey(new Uint8Array([/* secret key */]));
const embeddedWallet = Keypair.generate();

// Build the registration transaction
const { transaction, signers } = register.build(payer, {
  destination: 5555, // Your rollup's Hyperlane domain ID
  embedded_user: embeddedWallet.publicKey,
});

// Send the transaction
const connection = new Connection("https://api.testnet.solana.com", "confirmed");
const signature = await sendAndConfirmTransaction(
  connection,
  transaction,
  signers,
  { commitment: "confirmed" }
);

console.log("Registration transaction confirmed:", signature);
console.log("Embedded wallet public key:", embeddedWallet.publicKey.toBase58());
```

### API Reference

#### `HyperlaneSolanaRegister`

**Constructor**

```typescript
constructor(programIds: ProgramIds)
```

- `programIds.register` - The Hyperlane register program ID
- `programIds.mailbox` - The Hyperlane mailbox program ID
- `programIds.splNoop` - (Optional) The SPL Noop program ID, defaults to `noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV`

**Methods**

```typescript
build(user: Keypair, params: RegisterParams): PreparedTransaction
```

Builds a registration transaction.

Parameters:
- `user` - The Solana keypair that will pay for and sign the transaction
- `params.destination` - The destination Hyperlane domain ID (your rollup)
- `params.embedded_user` - The public key of the embedded wallet to register

Returns:
- `transaction` - A Solana `Transaction` object ready to be sent
- `signers` - An array of signers (the payer and a unique message account)

### Important Notes

1. **Account Ordering**: The accounts in the transaction must be in a specific order. The library handles this automatically.

2. **Unique Message Account**: Each registration requires a unique message account (auto-generated by the library) to prevent replay attacks.

3. **Signers**: The transaction requires two signers:
   - The payer (the user's wallet)
   - The unique message account (generated automatically)

4. **Message ID**: After sending the transaction, the Solana program logs the Hyperlane message ID which can be used to track the message delivery.

---

## Security Considerations

### Trust Model

1. **Trusted Program**: The Solana program ID configured in the `SolanaDeployment` is TRUSTED. The program owner can upgrade the program, so ensure the program is either:
   - Non-upgradeable (authority revoked), OR
   - Controlled by a trusted governance mechanism

2. **Mailbox Trust**: The Solana program only trusts a specific Hyperlane mailbox program. This is validated on-chain at `solana/program/src/lib.rs:82-89`.

3. **ISM Verification**: The Sovereign module uses an ISM (Interchain Security Module) to verify message authenticity. Configure an appropriate ISM for your security requirements.

---

## Testing

### Rust Module Tests

```bash
cd /path/to/hyperlane-solana-register
cargo test
```

### Solana Program Tests

```bash
cd solana
cargo test
```

Program tests are located in `solana/program-tests/src/tests.rs`.

