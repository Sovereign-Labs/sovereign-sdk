# @sovereign-sdk/universal-wallet-wasm

[![npm version](https://img.shields.io/npm/v/@sovereign-sdk/universal-wallet-wasm.svg)](https://www.npmjs.com/package/@sovereign-sdk/universal-wallet-wasm)
[![CI](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml/badge.svg)](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml)

WebAssembly bindings for Sovereign SDK's universal wallet, providing schema validation and serialization utilities.

## Installation

```bash
npm install @sovereign-sdk/universal-wallet-wasm
```

## Features

- 🔄 JSON to [Borsh](https://borsh.io/) serialization of rollup types
- 📝 Schema validation
- 📄 Human-readable display of Borsh bytes

## Usage

```typescript
import { Schema, KnownTypeId } from '@sovereign-sdk/universal-wallet-wasm';

// Initialize schema from JSON outputted during rollup compilation
const schema = Schema.fromJSON(JSON.stringify(yourSchemaJson));

// Convert JSON to Borsh bytes
const runtimeCall = { value_setter: { set_many_values: [4, 6] } };
const borshBytes = schema.jsonToBorsh(
  schema.knownTypeIndex(KnownTypeId.RuntimeCall),
  JSON.stringify(runtimeCall)
);

// Display Borsh bytes as human-readable string
const displayed = schema.display(
  schema.knownTypeIndex(KnownTypeId.RuntimeCall),
  borshBytes
);
```

## Development Guide

This package uses [wasm-pack](https://rustwasm.github.io/wasm-pack/) to compile Rust code to WebAssembly and generate JavaScript bindings.

### Prerequisites

1. Install Rust (version 1.79 or later)
2. Install Node.js dependencies: `pnpm install`

### Build Commands

- `pnpm build` - Builds both Node.js and ESM targets
- `pnpm compile:node` - Builds Node.js target only
- `pnpm compile:esm` - Builds ESM target only
- `pnpm test` - Runs tests

### Project Structure

```
universal-wallet-wasm/
├── src/           # Rust source code
├── tests/         # TypeScript tests
├── dist/         # Compiled outputs (generated)
│   ├── node/     # Node.js target
│   └── esm/      # ESM target
└── Cargo.toml    # Rust dependencies
└── package.json  # NPM package configuration
```

### Making Changes

1. Edit Rust code in `src/lib.rs`
2. Run `pnpm build` to compile
3. Add tests in `tests/` directory
4. Run `pnpm test` to verify changes