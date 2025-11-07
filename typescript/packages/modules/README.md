# @sovereign-sdk/modules

[![npm version](https://img.shields.io/npm/v/@sovereign-sdk/modules.svg)](https://www.npmjs.com/package/@sovereign-sdk/modules)
[![CI](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml/badge.svg)](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml)

Convenient helpers for interacting with core Sovereign SDK modules.

## Installation

```bash
npm install @sovereign-sdk/modules
```

## Usage

### Bank Module

```typescript
import { BankModule } from "@sovereign-sdk/modules";
import { StandardRollup } from "@sovereign-sdk/web3";

const rollup = new StandardRollup({
  url: "https://your-rollup-node.com",
  // ... other config
});

const bank = new BankModule(rollup);

// Get balance for an address
const balance = await bank.getBalance(address, tokenAddress);
```

