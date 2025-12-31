## Deploying

This document outlines the steps required to deploy the application to the Solana network.

> [!IMPORTANT]
> The program MUST be built with the same SBF compiler version as the hyperlane Solana programs (1.14.20) to ensure compatibility. If this is not done, the program will fail at runtime with various errors.

### Prerequisites

- A funded Solana wallet with sufficient SOL to cover deployment costs.

### Building the Program

1. Install the Solana toolchain using the script provided in `./scripts/install-solana-1.14.20.sh`
2. Build the program using the script provided in `./scripts/build-programs.sh`

### Generating a Program ID

To generate a new program ID, use the following command:

```sh
solana-keygen new --outfile ./solana/target/deploy/hyperlane_solana_sovereign_register-keypair.json
```

Now update the `declare_id!` macro in `solana/program/src/lib.rs` to match the newly generated program ID.

This value was outputted when you ran the `solana-keygen new` command above:

```sh
===============================================================================
pubkey: DEv8yXNmX7HiSWx48w4ScnY7KLDJBojsXA7yVTLMxQJX
===============================================================================
```

### Deploying the Program

We want the program to be deployed with a deterministic address. To achieve this, we use the `--program-id` flag with the `solana program deploy` command, specifying a pre-generated keypair file for the program ID.

Run the following command to deploy the program:

```sh
solana program deploy \
    --program-id ./solana/target/deploy/hyperlane_solana_sovereign_register-keypair.json \
    ./solana/target/deploy/hyperlane_solana_sovereign_register.so
```
