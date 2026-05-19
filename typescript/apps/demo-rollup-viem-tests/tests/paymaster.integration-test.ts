import { beforeAll, describe, expect, it } from "vitest";
import {
  encodeFunctionData,
  type Address,
  type Hex
} from "viem";
import {
  createDemoRollupRuntime,
  deploySimpleStorage,
  formatEstimateFailure,
  getEstimateMismatchAllowance,
  RECEIPT_TIMEOUT_MS
} from "../src/harness";

type Runtime = Awaited<ReturnType<typeof createDemoRollupRuntime>>;

interface Subcase {
  name: string;
  data: Hex;
  expectedLogCount?: number;
  verify: () => Promise<void>;
}

async function readSimpleStorageValue(
  runtime: Runtime,
  contractAddress: Address
): Promise<bigint> {
  return (await runtime.publicClient.readContract({
    address: contractAddress,
    abi: runtime.abi,
    functionName: "num"
  })) as bigint;
}

describe("demo-rollup paymaster-backed estimateGas", () => {
  let runtime: Runtime;
  let contractAddress: Address;

  beforeAll(async () => {
    runtime = await createDemoRollupRuntime();
    contractAddress = await deploySimpleStorage(runtime);
  });

  it("lets an unfunded sender estimate gas and execute paymaster-sponsored calls", async () => {
    const initialBalance = await runtime.publicClient.getBalance({
      address: runtime.unfunded.address
    });
    expect(initialBalance).toBe(0n);

    const subcases: Subcase[] = [
      {
        name: "set(7)",
        data: encodeFunctionData({
          abi: runtime.abi,
          functionName: "set",
          args: [7n]
        }),
        verify: async () => {
          expect(await readSimpleStorageValue(runtime, contractAddress)).toBe(7n);
        }
      },
      {
        name: "inc()",
        data: encodeFunctionData({
          abi: runtime.abi,
          functionName: "inc"
        }),
        verify: async () => {
          expect(await readSimpleStorageValue(runtime, contractAddress)).toBe(8n);
        }
      },
      {
        name: "emitLogs(123, 3)",
        data: encodeFunctionData({
          abi: runtime.abi,
          functionName: "emitLogs",
          args: [123n, 3n]
        }),
        expectedLogCount: 3,
        verify: async () => {
          expect(await readSimpleStorageValue(runtime, contractAddress)).toBe(8n);
        }
      }
    ];

    for (const subcase of subcases) {
      const nonce = await runtime.publicClient.getTransactionCount({
        address: runtime.unfunded.address,
        blockTag: "pending"
      });

      const estimateRequest = {
        account: runtime.unfunded.address,
        to: contractAddress,
        data: subcase.data,
        nonce
      } as const;

      let estimate: bigint;
      try {
        estimate = await runtime.publicClient.estimateGas(estimateRequest);
      } catch (error) {
        throw new Error(formatEstimateFailure(subcase.name, runtime.unfunded.address, error));
      }

      const hash = await runtime.unfundedWallet.sendTransaction({
        account: runtime.unfunded,
        to: contractAddress,
        data: subcase.data,
        gas: estimate,
        nonce
      });

      const receipt = await runtime.publicClient.waitForTransactionReceipt({
        hash,
        timeout: RECEIPT_TIMEOUT_MS
      });

      expect(receipt.status, subcase.name).toBe("success");
      const gasDelta =
        receipt.gasUsed > estimate ? receipt.gasUsed - estimate : estimate - receipt.gasUsed;
      expect(
        gasDelta,
        `${subcase.name} estimate should stay close to execution`
      ).toBeLessThanOrEqual(getEstimateMismatchAllowance(estimate));

      if (subcase.expectedLogCount !== undefined) {
        const matchingLogs = receipt.logs.filter(
          (log) => log.address.toLowerCase() === contractAddress.toLowerCase()
        );
        expect(matchingLogs, `${subcase.name} should emit the expected number of logs`).toHaveLength(
          subcase.expectedLogCount
        );
      }

      await subcase.verify();
    }

    const finalBalance = await runtime.publicClient.getBalance({
      address: runtime.unfunded.address
    });
    expect(finalBalance).toBe(0n);
  });
});
