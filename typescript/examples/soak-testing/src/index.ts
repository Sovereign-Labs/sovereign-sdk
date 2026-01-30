/**
 * Example implementation of a soak test using the @sovereign-sdk/test package.
 * This example demonstrates how to:
 * 1. Set up a rollup connection
 * 2. Create a custom transaction generator
 * 3. Configure and run a soak test
 *
 * The example performs bank transfer transactions between random accounts
 * and verifies the transaction events.
 */

import {
  createStandardRollup,
  type StandardRollupSpec,
} from "@sovereign-sdk/web3";
import {
  BasicGeneratorStrategy,
  TestRunner,
  type GeneratedInput,
  TransactionGenerator,
} from "@sovereign-sdk/test/soak";
import random from "random";
import keypairs from "../data/keypairs.json" assert { type: "json" };
import { Ed25519Signer } from "@sovereign-sdk/signers";
import { type RuntimeCall } from "./types";
import assert from "node:assert";

type Keypair = (typeof keypairs)[0];
type S = StandardRollupSpec<RuntimeCall>;

/**
 * Default transaction details used for all transactions in the soak test.
 * These values can be adjusted based on your rollup's requirements.
 */
const defaultTxDetails = {
  max_priority_fee_bips: 0,
  max_fee: "100000000",
  gas_limit: null,
  chain_id: 4321,
};

/**
 * The token ID used for gas payments in the test.
 * This should match your rollup's configuration.
 */
const gasTokenId =
  "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7";

/**
 * A transaction generator that produces bank transfer transactions.
 * Each generated transaction:
 * - Transfers a random amount between 5-100 tokens
 * - Uses random sender and receiver accounts
 * - Verifies the transaction events after submission
 *
 * Also contains 2 util functions to randomly select a sender & receiver as well as
 * converts a keypair into a `Signer`.
 *
 * @returns A TransactionGenerator instance that creates bank transfer transactions
 */
class BankTransferGenerator extends TransactionGenerator<S> {
  private readonly keypairs: Keypair[];

  constructor(keypairs: Keypair[]) {
    super();
    this.keypairs = keypairs;
  }

  /**
   * Performs the generation of a transaction that should succeed.
   *
   * Simply selects 2 random keypairs we generated previously (and set at genesis)
   * and transfers a randomly selected amount between them.
   */
  async successful(): Promise<GeneratedInput<S>> {
    const { sender, receiver } = this.getSenderAndReceiver();
    const amount = random.int(5, 100);
    const unsignedTransaction = {
      runtime_call: {
        bank: {
          transfer: {
            coins: {
              amount,
              token_id: gasTokenId,
            },
            to: receiver.address,
          },
        },
      },
      details: defaultTxDetails,
      generation: Date.now(),
    };

    return {
      unsignedTransaction,
      signer: new Ed25519Signer(sender.privateKey),
      async onSubmitted(result) {
        assert(
          result.events?.length === 1,
          "tranfer should only emit one event"
        );
        const event = result.events[0]?.value;

        assert.deepStrictEqual(
          event,
          {
            token_transferred: {
              from: {
                user: sender.address,
              },
              to: {
                user: receiver.address,
              },
              coins: {
                amount: amount.toString(),
                token_id: gasTokenId,
              },
            },
          },
          "transfer event should include the expected fields"
        );
      },
    };
  }

  /**
   * Randomly selects a sender and receiver from the available keypairs.
   * Ensures the sender and receiver are different accounts.
   *
   * @returns An object containing the selected sender and receiver keypairs
   */
  getSenderAndReceiver() {
    const sender = random.int(0, this.keypairs.length - 1);
    let receiver;

    do {
      receiver = random.int(0, this.keypairs.length - 1);
    } while (receiver === sender);

    return {
      sender: this.keypairs[sender]!,
      receiver: this.keypairs[receiver]!,
    };
  }
}

/**
 * Main function that sets up and runs the soak test.
 * 1. Creates a rollup connection
 * 2. Initializes the transaction generator
 * 3. Creates and runs the test runner
 *
 * The test will continue running until manually stopped.
 */
async function main(): Promise<void> {
  const rollup = await createStandardRollup<RuntimeCall>();

  const generator = new BasicGeneratorStrategy(
    new BankTransferGenerator(keypairs)
  );
  const runner = new TestRunner<S>({ rollup, generator });

  return runner.run();
}

main().catch(console.error);
