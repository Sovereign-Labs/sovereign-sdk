# @sovereign-sdk/signers

[![npm version](https://img.shields.io/npm/v/@sovereign-sdk/signers.svg)](https://www.npmjs.com/package/@sovereign-sdk/signers)
[![CI](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml/badge.svg)](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml)

A signer interface and implementations for use with Sovereign SDK applications.

`Ed25519Signer` and `PhantomSigner` sign the original message, preserving compatibility with existing Ed25519 rollups. For the Commonware demo, explicitly wrap a raw Ed25519 signer in `CommonwareSigner` to sign `0x00 || message`:

```typescript
import { CommonwareSigner, Ed25519Signer } from '@sovereign-sdk/signers';

const signer = new CommonwareSigner(new Ed25519Signer(privateKey));
const signature = await signer.sign(message);
```

The same wrapper supports raw software-wallet adapters, such as `new CommonwareSigner(new PhantomSigner(provider))`. Pass the original message and wrap the signer only once. Transaction and Solana envelope serialization remain unchanged. A Commonware signature verifies against the prefixed message; it is not interchangeable with an unprefixed signature.

Ledger Solana is excluded from this experiment: its firmware requires the Solana preamble at byte zero and rejects the Commonware prefix.

## Installation

```
npm install @sovereign-sdk/signers
```

## Features

- 🔐 Abstract `Signer` interface that can be implemented to provide custom signers for use in `@sovereign-sdk/web3`.
- 🦊 MetaMask Snap signer implementation for use with [Sovereign's Universal MetaMask Snap](https://github.com/Sovereign-Labs/sovereign-universal-snap)


## Usage

### MetaMask Snap Signer


```typescript
import { newMetaMaskSnapSigner } from '@sovereign-sdk/signers';

// Initialize the signer
const signer = newMetaMaskSnapSigner({
  curve: 'ed25519', // or 'secp256k1'
  schema: {
    // Your rollup schema
  },
  snapId: 'npm:@sovereign-sdk/metamask-snap', // Metamask snap origin, can be changed to `local:localhost:8080` for local development
});

// Get public key
const publicKey = await signer.publicKey();

// Sign a message
const message = new Uint8Array([/* your message */]);
const signature = await signer.sign(message);
```

### Custom Signer Implementation

You can implement your own signer by implementing the `Signer` interface if your wallet is not currently supported.

```typescript
import { Signer } from '@sovereign-sdk/signers';

class MySigner implements Signer {
  publicKey() {
    // retrieve the public key from your wallet
    return new Uint8Array([/* your public key */]);
  }

  async sign(message: Uint8Array) {
    // sign the message
    return new Uint8Array([/* your signature */]);
  }
}
```
