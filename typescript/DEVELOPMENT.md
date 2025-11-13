# Development Setup & Workflow

This document outlines our internal development practices, tools, and processes for our monorepo.

## Prerequisites

- Node.js LTS
- pnpm

## Monorepo Structure

Our monorepo contains multiple packages that are developed and versioned together. The repository is structured as follows:

```
root/
├── packages/
│   ├── signers/                    # Signers interface & default implementations
│   ├── universal-wallet-wasm/      # Universal wallet schema WASM bindings
│   ├── modules/                    # Helpers for interacting with core Sovereign SDK modules
│   ├── test/                       # Testing utilities and soak testing
│   ├── utils/                      # Common utilities and helper functions
│   └── web3/                       # Primary web3 package
├── apps/
│   └── indexer/                    # Standalone indexer application for Sovereign SDK rollups
├── examples/
│   └── soak-testing/               # Example soak testing implementation
├── docs/                           # Documentation
└── package.json                    # Root package.json
```

### Package Dependencies

Our packages follow this dependency hierarchy:

```
web3 -> signers, universal-wallet-wasm, utils
modules -> web3, utils
test -> (no dependencies within workspace)
utils -> (no dependencies within workspace)
signers -> (no dependencies within workspace)
universal-wallet-wasm -> (no dependencies within workspace)
```

- No circular dependencies are allowed
- web3 is generally the main package consumed by the outside world
- modules provides convenient helpers that build on top of web3
- utils, signers, and universal-wallet-wasm are foundational packages

### Working with Packages

The following commands are available at the root of the monorepo or you can run them in a specific package directory.

Navigate between packages:

```bash
# Direct navigation
cd packages/web3
cd packages/signers
cd packages/universal-wallet-wasm
```

Install all dependencies:

```bash
pnpm install
```

Build all packages:

```bash
pnpm build
```

Build specific packages:

```bash
pnpm run build --filter @sovereign-sdk/web3
pnpm run build --filter @sovereign-sdk/signers
pnpm run build --filter @sovereign-sdk/universal-wallet-wasm
```

Run tests:

```bash
# Test all packages
pnpm test

# Test specific packages
pnpm test --filter @sovereign-sdk/web3
pnpm test --filter @sovereign-sdk/universal-wallet-wasm
```

Formatting and linting:

```bash
# Fix linting and formatting errors
pnpm run fix

# Lint all packages
pnpm run lint
```

Adding dependencies:

```bash
# Add to a specific package
pnpm add <package> --filter @sovereign-sdk/web3

# Add as dev dependency
pnpm add -D <package> --filter @sovereign-sdk/web3

# Add workspace dependency
# Add signers package in this monorepo as a dependency to web3 package
pnpm add @sovereign-sdk/signers --filter @sovereign-sdk/web3 --workspace
```

## Changesets in Monorepo

We use changesets to manage versions, releases & changelogs across all packages.

If a change you are making should result in a new version of a package, you should create a changeset and include it in the PR.

The `changeset` bot will comment on your PR if it is missing a changeset, if your change doesn't impact a package (for example if you are just fixing a linting error) you can safely ignore the bot.

### Creating a changeset

The following commands are available at the root of the monorepo.

1. Create a changeset:

   ```bash
   pnpm run changeset
   ```

2. Select affected packages:
   - Choose all impacted packages

3. Enter a changeset message:
   - For example:
     - `Update button component`
     - `Add new feature X`

After running the command, you will see a changeset file created in the `.changesets` directory. After merging the PR a release PR will be created shortly after by our release bot. This PR will automatically update the version of the package and add a changelog entry. On merging of the release PR a new version of the package will be published to NPM.

Visit [Changesets](https://github.com/changesets/changesets) for more information.