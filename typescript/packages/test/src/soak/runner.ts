import type { BaseTypeSpec, Rollup } from "@sovereign-sdk/web3";
import PQueue from "p-queue";
import type { GeneratorStrategy } from "./generation";

/**
 * Configuration options for creating a new TestRunner instance.
 * @template S - The type specification for the rollup being tested
 */
export interface TestOptions<S extends BaseTypeSpec> {
  /** Maximum number of concurrent transactions to run (default: 30) */
  concurrency?: number;
  /** Strategy for generating test transactions */
  generator: GeneratorStrategy<S>;
  /** The rollup instance to test against */
  // biome-ignore lint/suspicious/noExplicitAny: generic not need
  rollup: Rollup<S, any>;
}

/**
 * A class for running soak tests against a Sovereign rollup.
 * Manages concurrent transaction execution and provides control over test execution.
 *
 * @template S - The type specification for the rollup being tested
 *
 * @example
 * ```typescript
 * const runner = new TestRunner({
 *   rollup: new YourRollup(),
 *   generator: new YourGeneratorStrategy(),
 *   concurrency: 30
 * });
 *
 * // Start the test
 * await runner.run();
 *
 * // Stop the test when needed
 * await runner.stop();
 * ```
 */
export class TestRunner<S extends BaseTypeSpec> {
  private controller: AbortController;
  private readonly concurrency: number;
  private readonly queue: PQueue;
  // biome-ignore lint/suspicious/noExplicitAny: generic not need
  private readonly rollup: Rollup<S, any>;
  private readonly generator: GeneratorStrategy<S>;

  /**
   * Creates a new TestRunner instance.
   * @param options - Configuration options for the test runner
   */
  constructor({ rollup, generator, concurrency }: TestOptions<S>) {
    this.controller = new AbortController();
    this.concurrency = concurrency ?? 30;
    this.queue = new PQueue({ concurrency: this.concurrency });
    this.rollup = rollup;
    this.generator = generator;
  }

  /**
   * Starts the soak test. This will begin generating and submitting transactions
   * according to the configured generator strategy and concurrency settings.
   *
   * The test will continue running until explicitly stopped using the stop() method.
   */
  async run(): Promise<void> {
    await this.silenceNodeWarning();
    this.doExecution();
  }

  /**
   * Stops the soak test. This will abort any pending transactions and prevent
   * new transactions from being generated.
   */
  async stop(): Promise<void> {
    this.controller.abort();
  }

  private async doExecution(): Promise<void> {
    if (this.controller.signal.aborted) return;

    // create a child abort signal per batch because there's no way
    // to clean up abort listeners, this prevents them building up and leaking
    const controller = new AbortController();
    const signal = controller.signal;
    this.controller.signal.addEventListener("abort", controller.abort, {
      once: true,
    });

    const inputs = await this.generator.generate(100);
    // TODO: check expectedOutcome with what the rollup actually returns
    const execution = inputs.map(({ input }) => {
      return async () => {
        if (signal.aborted) return;

        const { signer, unsignedTransaction, onSubmitted } = input;
        // TODO: handle abort error
        const { response } = await this.rollup.signAndSubmitTransaction(
          unsignedTransaction,
          { signer },
          { signal },
        );

        if (onSubmitted) {
          // TODO: handle errors
          await onSubmitted(response);
        }
      };
    });

    this.queue.addAll(execution);
    await this.queue.onSizeLessThan(this.concurrency);

    // clean-up listener to prevent listener leak on top-level abort controller
    this.controller.signal.removeEventListener("abort", controller.abort);

    setTimeout(() => this.doExecution(), 100);
  }

  // Node emits a warning if there's more than 10 listeners by default
  // to try detect listener leaks. This is nodejs specific, we're OK to ignore it.
  private async silenceNodeWarning(): Promise<void> {
    if (process?.versions?.node) {
      const events = await import("node:events");
      // biome-ignore lint/suspicious/noExplicitAny: node specific
      (events as any).setMaxListeners(200);
    }
  }
}
