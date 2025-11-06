# Soak Testing Example

This example demonstrates how to use the `@sovereign-sdk/test` package to perform soak testing on a Sovereign rollup. Soak testing helps verify the stability and performance of your rollup under sustained load by continuously submitting transactions.

## Table of Contents

- [Setup Process](#setup-process)
  - [Keypair Generation](#keypair-generation)
  - [Genesis Configuration](#genesis-configuration)
  - [Rollup Initialization](#rollup-initialization)
- [Design Goals](#design-goals)
  - [Black Box Testing](#black-box-testing)
  - [Production-Ready Code](#production-ready-code)
  - [Minimal Abstractions](#minimal-abstractions)
- [Concepts](#concepts)
  - [Transaction Generators](#transaction-generators)
  - [Generator Strategies](#generator-strategies)
- [Implementation Guide](#implementation-guide)
  - [Create a Transaction Generator](#1-create-a-transaction-generator)
  - [Choose or Create a Generator Strategy](#2-choose-or-create-a-generator-strategy)
  - [Configure and Run the Test](#3-configure-and-run-the-test)
- [Example Implementation](#example-implementation)

## Setup Process

This example is set up with a specific configuration to enable testing with real accounts and transactions. Here's how it works:

### Keypair Generation
The example generates sets of keypairs that are used for signing transactions during the soak test. These keypairs are created programmatically, providing the necessary private keys for transaction signing. This ensures that we have a pool of valid accounts to work with during testing.

The keypair generation is implemented in [`scripts/prepare.mjs`](scripts/prepare.mjs), which creates a configurable number of keypairs and stores them for use in the test.

### Genesis Configuration
The setup uses a base genesis file as a template, which is then populated with the generated keypairs. Each generated account is given an initial bank balance in the genesis state. This ensures that all test accounts have sufficient funds to perform transactions during the soak test.

The base genesis configuration is defined in [`templates/genesis.json`](templates/genesis.json). The script [`scripts/prepare.mjs`](scripts/prepare.mjs) reads this template and inserts the generated keypairs with their initial balances into the genesis state. The resulting genesis file is written to `data/genesis.json`, which is then used by the rollup when it starts up.

### Rollup Initialization
The rollup binary is started using:
1. The generated `genesis.json` file that contains the initial state with funded accounts
2. A rollup configuration file located in this directory that specifies the rollup's runtime config

This setup allows the soak test to run with real accounts and transactions, providing a more accurate representation of how the rollup will perform under load in a production environment.

## Design Goals

The soak testing framework was designed with several key principles in mind:

### Black Box Testing
The framework treats the rollup as a black box, allowing developers to focus on their transaction generation and fuzzing logic. This separation of concerns makes it easier to write effective tests and maintain them over time.

### Production-Ready Code
We believe in using real, production-ready code for testing. The same code you use to create transactions in your production applications can be used to generate soak test transactions. This approach:
- Reduces the learning curve for developers
- Ensures test behavior matches production behavior
- Makes it easier to maintain tests as your application evolves

### Minimal Abstractions
The framework aims to keep abstractions to a minimum, preferring to use familiar patterns and tools:
- Use standard genesis configurations for initial state
- Work with real transaction types and structures
- Leverage existing rollup client code
- Minimize the need for test-specific wrappers or utilities

While we strive to keep the framework simple and minimal, we're open to adding features that would be genuinely useful to developers. If you find yourself needing additional functionality, please let us know - we're happy to consider adding features that would make the framework more effective for real-world use cases.

## Concepts

### Transaction Generators

A transaction generator is responsible for creating individual test transactions. It implements the `TransactionGenerator` interface and should be designed to:

1. Create transactions for your rollup
2. Handle transaction signing
3. Verify transaction results
4. Optionally implement failure scenarios

Here's a basic example of a transaction generator:

```typescript
import { TransactionGenerator } from "@sovereign-sdk/test/soak";

class CustomGenerator implements TransactionGenerator<YourTypeSpec> {
  // Generates a transaction that should be successful
  async successful(): Promise<GeneratedInput<YourTypeSpec>> {
    // Create your transaction
    const unsignedTransaction = {
      // Your transaction details
    };

    // Get a signer for the transaction
    const signer = await this.getSigner();

    return {
      unsignedTransaction,
      signer,
      // Optional callback to verify transaction results
      onSubmitted: async (result) => {
        // Verify the transaction was successful
        // Check emitted events
        // Validate state changes
      },
    };
  }

  // Generates a transaction that should fail
  async failure(): Promise<GeneratedInput<YourTypeSpec>> {
    // Create a transaction that should fail
    const unsignedTransaction = {
      // Your invalid transaction details
    };

    const signer = await this.getSigner();

    return {
      unsignedTransaction,
      signer,
      onSubmitted: async (result) => {
        // Verify the transaction failed as expected
        // Check error messages
        // Validate state remains unchanged
      },
    };
  }
}
```

### Generator Strategies

A generator strategy determines how multiple transactions are generated and executed. The `BasicGeneratorStrategy` provided by the package:

1. Takes a single transaction generator
2. Creates multiple transactions in parallel
3. Always generates successful transactions

This is a good default if you don't need more advanced generation logic.

You might want to use a custom strategy to do a weighted select of many `TransactionGenerator`s to submit more of a particular type of call message for example.
The strategy could also be configured to generate occasional failure transactions to execerise more code paths in your application.

You can create custom strategies by implementing the `GeneratorStrategy` interface:

```typescript
import { GeneratorStrategy, Outcome } from "@sovereign-sdk/test/soak";

class CustomStrategy implements GeneratorStrategy<YourTypeSpec> {
  private readonly generators: TransactionGenerator<YourTypeSpec>[];

  constructor(generators: TransactionGenerator<YourTypeSpec>[]) {
    this.generators = generators;
  }

  async generate(amount: number): Promise<InputWithExpectation<YourTypeSpec>[]> {
    const transactions = [];

    for (let i = 0; i < amount; i++) {
      // Choose a generator based on your strategy
      const generator = this.selectGenerator();
      // Only generate successful transactions for this example
      // But you could randomly insert transactions that should fail
      const transaction = await generator.successful();
      transactions.push({input: transaction, expectedOutcome: Outcome.Success});
    }

    return transactions;
  }

  private selectGenerator(): TransactionGenerator<YourTypeSpec> {
    // Implement your selection logic
    // Could be random, round-robin, or based on some other criteria
    return this.generators[Math.floor(Math.random() * this.generators.length)];
  }
}
```

## Implementation Guide

### 1. Create a Transaction Generator

Start by implementing a transaction generator for your specific use case:

```typescript
class MyTransactionGenerator implements TransactionGenerator<S> {
  async successful(): Promise<GeneratedInput<S>> {
    // 1. Prepare transaction data
    const transactionData = {
      // Your transaction details
    };

    // 2. Get a signer
    const signer = await getSigner();

    // 3. Create the transaction
    const unsignedTransaction = {
      runtime_call: transactionData,
      details: defaultTxDetails,
      generation: Date.now(),
    };

    // 4. Return the transaction with optional verification
    return {
      unsignedTransaction,
      signer,
      onSubmitted: async (result) => {
        // Verify transaction results
        // Check events
        // Validate state changes
      },
    };
  }
}
```

### 2. Choose or Create a Generator Strategy

Use the `BasicGeneratorStrategy` for simple cases:

```typescript
const generator = new BasicGeneratorStrategy(myTransactionGenerator());
```

Or create a custom strategy for more complex scenarios:

```typescript
class ComplexStrategy implements GeneratorStrategy<S> {
  private readonly generators: TransactionGenerator<S>[];

  constructor() {
    this.generators = [
      transferGenerator(),
      stakeGenerator(),
      unstakeGenerator(),
      // Add more generators as needed
    ];
  }

  async generate(amount: number): Promise<GeneratedInput<S>[]> {
    // Implement your strategy for generating multiple transactions
  }
}
```

### 3. Configure and Run the Test

Set up the test runner with your strategy:

```typescript
const rollup = await createStandardRollup<YourTypeSpec>({
  context: { defaultTxDetails },
});

const strategy = new YourStrategy();
const runner = new TestRunner<S>({
  rollup,
  generator: strategy,
  concurrency: 30, // Adjust based on your needs
});

// Start the test
await runner.run();

// Stop when needed
await runner.stop();
```

## Example Implementation

The included example demonstrates a bank transfer soak test that:

- Transfers random amounts between accounts
- Verifies transfer events
- Uses the `BasicGeneratorStrategy` for simplicity

You can use this as a starting point for your own implementations.
