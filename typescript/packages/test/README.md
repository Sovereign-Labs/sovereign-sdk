# @sovereign-sdk/test

Testing utilities for Sovereign SDK rollups, including soak testing and transaction generation.

## Installation

```bash
npm install @sovereign-sdk/test
```

## Usage

The package provides utilities for testing Sovereign SDK rollups, with a focus on soak testing and transaction generation. Soak testing helps verify the stability and performance of your rollup under sustained load by continuously submitting transactions.

> **Note:** Currently, soak tests require a pre-started rollup node to run against. In a future version, we plan to add functionality to manage the rollup process lifecycle (starting, stopping, and monitoring the node) as part of the test framework.

### Transaction Generators

Transaction generators are responsible for creating individual test transactions. They implement the `TransactionGenerator` interface:

```typescript
import { TransactionGenerator } from "@sovereign-sdk/test/soak";

class MyTransactionGenerator implements TransactionGenerator<YourTypeSpec> {
  async successful(): Promise<GeneratedInput<YourTypeSpec>> {
    // Create a transaction that should succeed
    const unsignedTransaction = {
      // Your transaction details
    };

    const signer = await getSigner();

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

  async failure(): Promise<GeneratedInput<YourTypeSpec>> {
    // Create a transaction that should fail
    const unsignedTransaction = {
      // Your invalid transaction details
    };

    const signer = await getSigner();

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

Generator strategies determine how multiple transactions are generated and executed. The package provides a `BasicGeneratorStrategy` that generates a fixed number of successful transactions:

```typescript
import { BasicGeneratorStrategy } from "@sovereign-sdk/test/soak";

const generator = new MyTransactionGenerator();
const strategy = new BasicGeneratorStrategy(generator);

// Generate 100 transactions
const transactions = await strategy.generate(100);
```

You can also create custom strategies by implementing the `GeneratorStrategy` interface:

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
      const transaction = await generator.successful();
      transactions.push({
        input: transaction,
        expectedOutcome: Outcome.Success
      });
    }

    return transactions;
  }

  private selectGenerator(): TransactionGenerator<YourTypeSpec> {
    // Implement your selection logic
    return this.generators[Math.floor(Math.random() * this.generators.length)];
  }
}
```

### Running Soak Tests

To run a soak test, create a test runner with your strategy:

```typescript
import { TestRunner } from "@sovereign-sdk/test/soak";

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

> **Note:** The current API requires manual management of the test runner lifecycle (start/stop). In a future version, we plan to simplify this by handling the lifecycle internally, so users won't need to manage these operations in their code.

## Example

See the [soak-testing example](../../examples/soak-testing) for a complete implementation of a bank transfer soak test.

## API Reference

### TransactionGenerator

Interface for generating individual test transactions.

```typescript
abstract class TransactionGenerator<S extends BaseTypeSpec> {
  abstract successful(): Promise<GeneratedInput<S>>;
  abstract failure(): Promise<GeneratedInput<S>>;
}
```

### GeneratorStrategy

Interface for generating batches of test transactions.

```typescript
interface GeneratorStrategy<S extends BaseTypeSpec> {
  generate(amount: number): Promise<InputWithExpectation<S>[]>;
}
```

### TestRunner

Class for running soak tests.

```typescript
class TestRunner<S extends BaseTypeSpec> {
  constructor(options: {
    rollup: StandardRollup<S>;
    generator: GeneratorStrategy<S>;
    concurrency: number;
  });

  run(): Promise<void>;
  stop(): Promise<void>;
}
```

