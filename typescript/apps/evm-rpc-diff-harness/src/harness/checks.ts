import { setTimeout as delay } from "node:timers/promises";
import {
  AbiCoder,
  Contract,
  Interface,
  isAddress,
  keccak256,
  toBeHex
} from "ethers";
import {
  createContract,
  sendValueTransactionWithFallback,
  sendContractTransactionWithFallback,
  toRpcError
} from "./deploy";
import {
  deepEqualNormalized,
  minimalJsonDiff,
  normalizeForComparison,
  validateBlockShape
} from "./normalize";
import { JsonRpcClient, isNotSupportedError } from "./rpc";
import type {
  CheckResult,
  EndpointExecution,
  EndpointObservation,
  EndpointRuntime,
  HarnessContext,
  Outcome,
  RpcErrorShape
} from "./types";

interface ComparisonResult {
  outcome: Outcome;
  diff?: unknown;
  notes?: string[];
}

interface CheckDefinition {
  name: string;
  library: "ethers" | "viem" | "raw";
  rpcMethods: string[];
  runEndpoint: (runtime: EndpointRuntime, context: HarnessContext) => Promise<EndpointExecution>;
  compare?: (anvil: EndpointObservation, rollup: EndpointObservation) => ComparisonResult;
}

interface DecodedHarnessLogEntry {
  event: string;
  args?: unknown;
  txHash: string | null;
  topics?: unknown;
  data?: string;
}

interface LogsFiltersCheckObservation {
  targetTxHash: string;
  byAddress: DecodedHarnessLogEntry[];
  byTopic0: DecodedHarnessLogEntry[];
  byIndexedTopic: DecodedHarnessLogEntry[];
  byRange: DecodedHarnessLogEntry[];
}

type ErrorConventionCase =
  | { ok: true }
  | {
      ok: false;
      code: number | null;
      message: string;
      hasMessage: boolean;
      hasData: boolean;
      data: unknown;
      raw: RpcErrorShape;
    };

interface EstimateGasObservation {
  setSimple: { values: string[]; above21000: boolean; stabilitySpread: number };
  emitMultipleEvents: { values: string[]; above21000: boolean; stabilitySpread: number };
  revertEstimate: { failed: boolean };
}

interface BlockShapeObservation {
  latestHashesShape: { ok: boolean; issues: string[] };
  latestFullShape: { ok: boolean; issues: string[] };
}

interface BlockNumberConsistencyObservation {
  checks: { nonDecreasing: boolean; latestWithinRange: boolean };
  blockNumberA: { validHexQuantity: boolean };
  latestBlockNumber: { validHexQuantity: boolean };
  blockNumberB: { validHexQuantity: boolean };
}

interface BlockTagStateReadsObservation {
  blocks: { historicalComparable: boolean; sameBlock: boolean };
  checks: {
    atSecondMatchesExpected: boolean;
    latestMatchesExpected: boolean;
    atFirstMatchesExpectedWhenComparable: boolean | null;
    pendingHasMessageIfErrored: boolean;
  };
}

interface PendingToSealedObservation {
  checks: {
    txFoundEventually: boolean;
    receiptFoundEventually: boolean;
    receiptHasStatus: boolean;
    receiptHashMatches: boolean;
  };
}

interface BlockLookupObservation {
  shapes: {
    latestByNumber: { ok: boolean };
    byHashShort: { ok: boolean };
    byHashFull: { ok: boolean };
  };
  linkage: { hashMatchesByHash: boolean; numberMatchesByHash: boolean; hashMatchesFull: boolean };
  txCount: {
    supported: boolean;
    validHex: boolean;
    equalsEachOther: boolean;
    matchesLatestArrayLength: boolean;
    unsupportedError?: RpcErrorShape;
  };
  invalidHashBehavior: { returnedNull: boolean; error: RpcErrorShape | null };
}

interface FeeHistoryObservation {
  shape: {
    oldestBlockHex: boolean;
    baseFeePerGasLength: number;
    baseFeePerGasAllHex: boolean;
    gasUsedRatioLength: number;
    gasUsedRatioAllNumbers: boolean;
    rewardShapeValid: boolean;
  };
  invalidTag: { ok: boolean; hasMessage?: boolean; message?: string };
}

interface BlockReceiptsObservation {
  shapeOk: boolean;
  countMatchesBlock: boolean;
}

function normalizeAddress(address: string, runtime: EndpointRuntime): string {
  const lower = address.toLowerCase();
  if (lower === runtime.wallet.address.toLowerCase()) {
    return "<TEST_ACCOUNT>";
  }
  if (lower === runtime.deployment.kitchenSink.toLowerCase()) {
    return "<KITCHEN_SINK>";
  }
  if (lower === runtime.deployment.delegateTarget.toLowerCase()) {
    return "<DELEGATE_TARGET>";
  }
  if (lower === runtime.deployment.callReceiver.toLowerCase()) {
    return "<CALL_RECEIVER>";
  }
  return lower;
}

function normalizeRuntimeValue(value: unknown, runtime: EndpointRuntime): unknown {
  if (typeof value === "bigint") {
    return value.toString();
  }

  if (typeof value === "string") {
    if (isAddress(value)) {
      return normalizeAddress(value, runtime);
    }
    return value;
  }

  if (Array.isArray(value)) {
    return value.map((item) => normalizeRuntimeValue(item, runtime));
  }

  if (value && typeof value === "object") {
    const obj = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(obj)) {
      out[key] = normalizeRuntimeValue(obj[key], runtime);
    }
    return out;
  }

  return value;
}

function namedArgs(args: unknown): Record<string, unknown> {
  if (!args || typeof args !== "object") {
    return {};
  }

  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args as Record<string, unknown>)) {
    if (!Number.isNaN(Number(key))) {
      continue;
    }
    out[key] = value;
  }
  return out;
}

function defaultComparison(anvil: EndpointObservation, rollup: EndpointObservation): ComparisonResult {
  if (rollup.unsupported || isNotSupportedError(rollup.error)) {
    return {
      outcome: "NOT_SUPPORTED",
      diff: {
        rollupError: rollup.error
      }
    };
  }

  if (anvil.error || rollup.error) {
    return {
      outcome: "FAIL",
      diff: {
        anvilError: anvil.error,
        rollupError: rollup.error
      }
    };
  }

  if (deepEqualNormalized(anvil.normalized, rollup.normalized)) {
    return { outcome: "PASS" };
  }

  return {
    outcome: "FAIL",
    diff: minimalJsonDiff(anvil.normalized, rollup.normalized)
  };
}

function standardCompare<T>(
  anvil: EndpointObservation,
  rollup: EndpointObservation,
  validate: (left: T, right: T) => string[] | ComparisonResult
): ComparisonResult {
  if (rollup.unsupported || isNotSupportedError(rollup.error)) {
    return { outcome: "NOT_SUPPORTED", diff: { rollupError: rollup.error } };
  }
  if (anvil.error || rollup.error || !anvil.normalized || !rollup.normalized) {
    return { outcome: "FAIL", diff: { anvilError: anvil.error, rollupError: rollup.error } };
  }
  const result = validate(anvil.normalized as T, rollup.normalized as T);
  if (Array.isArray(result)) {
    if (result.length === 0) return { outcome: "PASS" };
    return { outcome: "FAIL", diff: { failures: result, anvil: anvil.normalized, rollup: rollup.normalized } };
  }
  return result;
}

function buildFailureExecution(error: unknown): EndpointExecution {
  return {
    observation: {
      error: toRpcError(error),
      raw: {
        message: error instanceof Error ? error.message : String(error)
      }
    },
    requests: []
  };
}

async function runCheck(definition: CheckDefinition, context: HarnessContext): Promise<CheckResult> {
  const anvilExecution = await definition
    .runEndpoint(context.anvil, context)
    .catch((error) => buildFailureExecution(error));
  const rollupExecution = await definition
    .runEndpoint(context.rollup, context)
    .catch((error) => buildFailureExecution(error));

  const comparison = definition.compare
    ? definition.compare(anvilExecution.observation, rollupExecution.observation)
    : defaultComparison(anvilExecution.observation, rollupExecution.observation);

  return {
    name: definition.name,
    library: definition.library,
    rpcMethods: definition.rpcMethods,
    requests: {
      anvil: anvilExecution.requests,
      rollup: rollupExecution.requests
    },
    anvil: {
      normalized: normalizeForComparison(anvilExecution.observation.normalized),
      raw: anvilExecution.observation.raw,
      error: anvilExecution.observation.error,
      unsupported: anvilExecution.observation.unsupported,
      notes: anvilExecution.observation.notes
    },
    rollup: {
      normalized: normalizeForComparison(rollupExecution.observation.normalized),
      raw: rollupExecution.observation.raw,
      error: rollupExecution.observation.error,
      unsupported: rollupExecution.observation.unsupported,
      notes: rollupExecution.observation.notes
    },
    outcome: comparison.outcome,
    diff: comparison.diff,
    notes: comparison.notes
  };
}

function getRpcError(error: unknown): RpcErrorShape {
  return toRpcError(error);
}

async function sendValueWithFallback(
  runtime: EndpointRuntime,
  to: string,
  value: bigint
): Promise<{ txHash: string; status: number; txTypeUsed: "eip1559" | "legacy" }> {
  const result = await sendValueTransactionWithFallback(runtime, to, value);
  return {
    txHash: result.txHash,
    status: result.receipt.status,
    txTypeUsed: result.txTypeUsed
  };
}

function decodeReceiptLogs(
  runtime: EndpointRuntime,
  iface: Interface,
  logs: Array<{ topics: readonly string[]; data: string; address: string }>
): unknown[] {
  const decoded: unknown[] = [];
  for (const log of logs) {
    try {
      const parsed = iface.parseLog(log);
      decoded.push({
        event: parsed?.name,
        args: normalizeRuntimeValue(namedArgs(parsed?.args), runtime)
      });
    } catch {
      // Ignore logs for unrelated contracts.
    }
  }
  return decoded;
}

function extractRevertData(error: unknown): string | null {
  const stack: unknown[] = [error];
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current || typeof current !== "object") {
      continue;
    }

    const obj = current as Record<string, unknown>;

    if (typeof obj.data === "string" && /^0x[0-9a-fA-F]*$/.test(obj.data)) {
      return obj.data;
    }

    for (const key of ["error", "info", "cause", "value", "result"]) {
      if (obj[key] !== undefined) {
        stack.push(obj[key]);
      }
    }
  }

  return null;
}

function hasMessage(error: unknown): boolean {
  if (!error) {
    return false;
  }
  if (error instanceof Error) {
    return error.message.length > 0;
  }
  if (typeof error === "object") {
    const obj = error as { message?: unknown; shortMessage?: unknown };
    return typeof obj.message === "string" || typeof obj.shortMessage === "string";
  }
  return false;
}

function toHexQuantity(value: number): string {
  return `0x${value.toString(16)}`;
}

function parseHexQuantity(value: unknown): bigint | null {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]+$/.test(value)) {
    return null;
  }

  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

function toErrorConventionCase(call: { ok: boolean; error?: RpcErrorShape }): ErrorConventionCase {
  if (call.ok || !call.error) {
    return { ok: true };
  }

  return {
    ok: false,
    code: call.error.code,
    message: call.error.message,
    hasMessage: typeof call.error.message === "string" && call.error.message.length > 0,
    hasData: call.error.data !== undefined,
    data: call.error.data,
    raw: call.error
  };
}

function decodeUintFromCallResult(
  iface: Interface,
  method: string,
  result: unknown
): { ok: true; value: string } | { ok: false; reason: string } {
  if (typeof result !== "string") {
    return { ok: false, reason: "eth_call result is not a hex string" };
  }

  try {
    const decoded = iface.decodeFunctionResult(method, result);
    const first = decoded[0];
    if (typeof first === "bigint") {
      return { ok: true, value: first.toString() };
    }
    return { ok: false, reason: "Decoded value is not uint256" };
  } catch (error) {
    return {
      ok: false,
      reason: error instanceof Error ? error.message : String(error)
    };
  }
}

function isLikely1559Unsupported(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  const normalized = message.toLowerCase();
  return (
    normalized.includes("1559") ||
    normalized.includes("maxfeepergas") ||
    normalized.includes("transaction type") ||
    normalized.includes("unsupported transaction")
  );
}

const checks: CheckDefinition[] = [
  {
    name: "A.tx_lifecycle_ethers",
    library: "ethers",
    rpcMethods: ["eth_sendRawTransaction", "eth_getTransactionByHash", "eth_getTransactionReceipt"],
    runEndpoint: async (runtime, context) => {
      const iface = new Interface(context.contracts.kitchenSink.abi);
      const txResult = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "emitMultipleEvents",
        [777n, "tx-lifecycle", "0xabcdef"]
      );

      const [tx, receipt] = await Promise.all([
        runtime.provider.getTransaction(txResult.txHash),
        runtime.provider.getTransactionReceipt(txResult.txHash)
      ]);

      if (!tx || !receipt) {
        throw new Error("Failed to load sent transaction details");
      }

      const decodedLogs = decodeReceiptLogs(
        runtime,
        iface,
        receipt.logs.map((log) => ({
          topics: log.topics,
          data: log.data,
          address: log.address
        }))
      );

      return {
        observation: {
          normalized: {
            txTypeUsed: txResult.txTypeUsed,
            receiptStatus: Number(receipt.status ?? 0),
            decodedLogs,
            receiptHasEffectiveGasPrice:
              (receipt as { effectiveGasPrice?: bigint | null; gasPrice?: bigint | null })
                .effectiveGasPrice !== undefined ||
              (receipt as { effectiveGasPrice?: bigint | null; gasPrice?: bigint | null })
                .gasPrice !== undefined,
            deploymentReceiptContractAddressPresent:
              runtime.deployment.deploymentReceiptContractAddressPresent,
            consistency: {
              txHashMatches: tx.hash.toLowerCase() === txResult.txHash.toLowerCase(),
              receiptHashMatches: receipt.hash.toLowerCase() === txResult.txHash.toLowerCase(),
              receiptBlockHashMatchesTx:
                tx.blockHash !== null &&
                receipt.blockHash !== null &&
                tx.blockHash.toLowerCase() === receipt.blockHash.toLowerCase(),
              receiptBlockNumberMatchesTx: tx.blockNumber !== null && tx.blockNumber === receipt.blockNumber
            }
          }
        },
        requests: [
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "emitMultipleEvents",
              args: ["777", "tx-lifecycle", "0xabcdef"]
            }
          },
          {
            method: "eth_getTransactionByHash",
            params: [txResult.txHash]
          },
          {
            method: "eth_getTransactionReceipt",
            params: [txResult.txHash]
          }
        ]
      };
    },
    compare: (anvil, rollup) => {
      const fallback = defaultComparison(anvil, rollup);
      if (fallback.outcome !== "PASS") {
        if (fallback.outcome === "FAIL" && !anvil.error && !rollup.error) {
          const a = (anvil.normalized ?? {}) as Record<string, unknown>;
          const b = (rollup.normalized ?? {}) as Record<string, unknown>;
          const comparableA = { ...a, txTypeUsed: null };
          const comparableB = { ...b, txTypeUsed: null };
          if (deepEqualNormalized(comparableA, comparableB)) {
            return {
              outcome: "PASS",
              notes: [
                `Transaction type differs (anvil=${String(a.txTypeUsed)} rollup=${String(
                  b.txTypeUsed
                )}), but lifecycle checks matched.`
              ]
            };
          }
        }
        return fallback;
      }
      return fallback;
    }
  },
  {
    name: "B.eth_call_scalars_ethers",
    library: "ethers",
    rpcMethods: ["eth_call"],
    runEndpoint: async (runtime, context) => {
      const kitchen = createContract(runtime, context.contracts.kitchenSink, runtime.deployment.kitchenSink);

      const [bundle, mappingValue, nestedValue] = await Promise.all([
        kitchen.getFunction("getScalarBundle").staticCall(),
        kitchen.getFunction("simpleMapping").staticCall(runtime.wallet.address),
        kitchen.getFunction("getNestedMapping").staticCall(runtime.wallet.address, 9n)
      ]);

      return {
        observation: {
          normalized: {
            scalarBundle: {
              unsignedValue: bundle[0].toString(),
              signedValue: bundle[1].toString(),
              addressValue: normalizeRuntimeValue(bundle[2], runtime),
              boolValue: bundle[3]
            },
            simpleMappingValue: mappingValue.toString(),
            nestedMappingValue: String(nestedValue)
          }
        },
        requests: [
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "getScalarBundle"
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "simpleMapping",
              args: ["<TEST_ACCOUNT>"]
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "getNestedMapping",
              args: ["<TEST_ACCOUNT>", "9"]
            }
          }
        ]
      };
    }
  },
  {
    name: "B.eth_call_dynamic_viem",
    library: "viem",
    rpcMethods: ["eth_call"],
    runEndpoint: async (runtime, context) => {
      const dynamic = (await runtime.viemClient.readContract({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "getDynamicBundle"
      })) as readonly [
        `0x${string}`,
        string,
        readonly bigint[],
        { id: bigint; label: string; blob: `0x${string}`; enabled: boolean }
      ];

      const roundtrip = (await runtime.viemClient.readContract({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "hashAndRoundtrip",
        args: [123n, "roundtrip", "0xa1b2c3"]
      })) as readonly [`0x${string}`, bigint, string, `0x${string}`];

      const packedHash = (await runtime.viemClient.readContract({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "encodePackedHash",
        args: [runtime.account.address, 123n, "0xa1b2c3"]
      })) as `0x${string}`;

      const sha = (await runtime.viemClient.readContract({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "precompileSha256",
        args: ["0x1234"]
      })) as `0x${string}`;

      const ripemd = (await runtime.viemClient.readContract({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "precompileRipemd160",
        args: ["0x1234"]
      })) as `0x${string}`;

      return {
        observation: {
          normalized: {
            dynamic: {
              bytesValue: dynamic[0],
              stringValue: dynamic[1],
              arrayValue: dynamic[2].map((item) => item.toString()),
              structValue: {
                id: dynamic[3].id.toString(),
                label: dynamic[3].label,
                blob: dynamic[3].blob,
                enabled: dynamic[3].enabled
              }
            },
            roundtrip: {
              digest: roundtrip[0],
              decodedValue: roundtrip[1].toString(),
              decodedText: roundtrip[2],
              decodedBytes: roundtrip[3]
            },
            packedHash,
            precompileSha256: sha,
            precompileRipemd160: ripemd
          }
        },
        requests: [
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "getDynamicBundle"
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "hashAndRoundtrip",
              args: ["123", "roundtrip", "0xa1b2c3"]
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "encodePackedHash",
              args: ["<TEST_ACCOUNT>", "123", "0xa1b2c3"]
            }
          }
        ]
      };
    }
  },
  {
    name: "A.special_paths_create2_delegatecall_ethers",
    library: "ethers",
    rpcMethods: ["eth_sendRawTransaction", "eth_call"],
    runEndpoint: async (runtime, context) => {
      const kitchen = createContract(runtime, context.contracts.kitchenSink, runtime.deployment.kitchenSink);
      const create2Salt = keccak256(Buffer.from("kitchen-sink-create2"));
      const create2InitValue = 31_337n;

      const delegateTx = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "delegateWrite",
        [runtime.deployment.delegateTarget, 909n]
      );

      const delegatedValue = await kitchen.getFunction("delegatedValue").staticCall();

      const abiCoder = AbiCoder.defaultAbiCoder();
      const constructorData = abiCoder.encode(["uint256"], [create2InitValue]);
      const creationCode = `${context.contracts.create2Child.bytecode}${constructorData.slice(2)}`;
      const codeHash = keccak256(creationCode);

      const expectedAddress = (await kitchen.getFunction("computeCreate2Address").staticCall(
        create2Salt,
        codeHash
      )) as string;

      const create2Tx = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "deployCreate2",
        [create2Salt, create2InitValue]
      );

      const create2Receipt = await runtime.provider.getTransactionReceipt(create2Tx.txHash);
      if (!create2Receipt) {
        throw new Error("Missing CREATE2 receipt");
      }

      const iface = new Interface(context.contracts.kitchenSink.abi);
      let deployedAddress: string | null = null;
      let expectedFromEvent: string | null = null;

      for (const log of create2Receipt.logs) {
        try {
          const parsed = iface.parseLog(log);
          if (parsed && parsed.name === "Create2Deployed") {
            expectedFromEvent = String(parsed.args.expected);
            deployedAddress = String(parsed.args.deployed);
          }
        } catch {
          // Ignore unrelated logs.
        }
      }

      if (!deployedAddress) {
        deployedAddress = expectedAddress;
      }

      const child = new Contract(deployedAddress, context.contracts.create2Child.abi, runtime.provider);
      const childInitValue = await child.getFunction("initValue").staticCall();
      const code = await runtime.provider.getCode(deployedAddress);

      return {
        observation: {
          normalized: {
            delegatecall: {
              txStatus: delegateTx.receipt.status,
              delegatedValue: delegatedValue.toString()
            },
            create2: {
              txStatus: create2Tx.receipt.status,
              expectedMatchesEvent:
                expectedFromEvent === null ||
                expectedFromEvent.toLowerCase() === expectedAddress.toLowerCase(),
              deployedMatchesExpected: deployedAddress.toLowerCase() === expectedAddress.toLowerCase(),
              deployedCodePresent: code !== "0x",
              childInitValue: childInitValue.toString()
            }
          }
        },
        requests: [
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "delegateWrite",
              args: ["<DELEGATE_TARGET>", "909"]
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "computeCreate2Address",
              args: [create2Salt, "<create2_code_hash>"]
            }
          },
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "deployCreate2",
              args: [create2Salt, create2InitValue.toString()]
            }
          }
        ]
      };
    }
  },
  {
    name: "A.eth_transfer_paths_ethers",
    library: "ethers",
    rpcMethods: ["eth_sendRawTransaction", "eth_call"],
    runEndpoint: async (runtime, context) => {
      const receiver = createContract(runtime, context.contracts.callReceiver, runtime.deployment.callReceiver);

      const amount = 100_000_000_000_000n;
      const beforeBalance = (await receiver.getFunction("getBalance").staticCall()) as bigint;
      const receiveTx = await sendValueWithFallback(runtime, runtime.deployment.kitchenSink, amount);

      const forwardTx = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "forwardEther",
        [runtime.deployment.callReceiver, amount]
      );

      const afterBalance = (await receiver.getFunction("getBalance").staticCall()) as bigint;

      return {
        observation: {
          normalized: {
            receiveStatus: receiveTx.status,
            forwardStatus: forwardTx.receipt.status,
            receiveTxType: receiveTx.txTypeUsed,
            forwardTxType: forwardTx.txTypeUsed,
            receiverBalanceDelta: (afterBalance - beforeBalance).toString(),
            expectedForwardAmount: amount.toString()
          }
        },
        requests: [
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              value: amount.toString(),
              note: "payable receive"
            }
          },
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "forwardEther",
              args: ["<CALL_RECEIVER>", amount.toString()]
            }
          }
        ]
      };
    },
    compare: (anvil, rollup) => {
      const fallback = defaultComparison(anvil, rollup);
      if (fallback.outcome !== "FAIL") {
        return fallback;
      }

      if (anvil.error || rollup.error) {
        return fallback;
      }

      const a = (anvil.normalized ?? {}) as Record<string, unknown>;
      const b = (rollup.normalized ?? {}) as Record<string, unknown>;
      const trimmedA = { ...a, receiveTxType: null, forwardTxType: null };
      const trimmedB = { ...b, receiveTxType: null, forwardTxType: null };

      if (deepEqualNormalized(trimmedA, trimmedB)) {
        return {
          outcome: "PASS",
          notes: [
            `Transaction type differs for ETH transfer path (anvil receive=${String(
              a.receiveTxType
            )}, rollup receive=${String(b.receiveTxType)}).`
          ]
        };
      }

      return fallback;
    }
  },
  {
    name: "A.revert_behavior_ethers",
    library: "ethers",
    rpcMethods: ["eth_call"],
    runEndpoint: async (runtime, context) => {
      const kitchen = createContract(runtime, context.contracts.kitchenSink, runtime.deployment.kitchenSink);
      const iface = new Interface(context.contracts.kitchenSink.abi);
      const customErrorSelector = iface.getError("CustomFailure")?.selector ?? null;

      async function capture(method: string, args: unknown[]): Promise<unknown> {
        try {
          await kitchen.getFunction(method).staticCall(...args);
          return {
            reverted: false,
            selector: null,
            hasMessage: false
          };
        } catch (error) {
          const data = extractRevertData(error);
          return {
            reverted: true,
            selector: data ? data.slice(0, 10).toLowerCase() : null,
            hasMessage: hasMessage(error)
          };
        }
      }

      return {
        observation: {
          normalized: {
            revertString: await capture("revertWithString", []),
            revertCustom: await capture("revertWithCustom", [501n]),
            revertPanic: await capture("revertWithPanic", []),
            expectedSelectors: {
              string: "0x08c379a0",
              custom: customErrorSelector,
              panic: "0x4e487b71"
            }
          }
        },
        requests: [
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "revertWithString"
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "revertWithCustom",
              args: ["501"]
            }
          },
          {
            method: "eth_call",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "revertWithPanic"
            }
          }
        ]
      };
    }
  },
  {
    name: "C.eth_estimateGas_viem",
    library: "viem",
    rpcMethods: ["eth_estimateGas"],
    runEndpoint: async (runtime, context) => {
      const estimateSimpleA = await runtime.viemClient.estimateContractGas({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "setSimpleValue",
        args: [7n],
        account: runtime.account.address
      });

      const estimateSimpleB = await runtime.viemClient.estimateContractGas({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "setSimpleValue",
        args: [8n],
        account: runtime.account.address
      });

      const estimateEventA = await runtime.viemClient.estimateContractGas({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "emitMultipleEvents",
        args: [123n, "estimate", "0x123456"],
        account: runtime.account.address
      });

      const estimateEventB = await runtime.viemClient.estimateContractGas({
        address: runtime.deployment.kitchenSink as `0x${string}`,
        abi: context.contracts.kitchenSink.abi as never,
        functionName: "emitMultipleEvents",
        args: [124n, "estimate", "0x123456"],
        account: runtime.account.address
      });

      let revertError: RpcErrorShape | undefined;
      try {
        await runtime.viemClient.estimateContractGas({
          address: runtime.deployment.kitchenSink as `0x${string}`,
          abi: context.contracts.kitchenSink.abi as never,
          functionName: "revertWithString",
          account: runtime.account.address
        });
      } catch (error) {
        revertError = getRpcError(error);
      }

      const simpleSpread =
        Number(estimateSimpleA > estimateSimpleB ? estimateSimpleA - estimateSimpleB : estimateSimpleB - estimateSimpleA) /
        Number(estimateSimpleA === 0n ? 1n : estimateSimpleA);

      const eventSpread =
        Number(estimateEventA > estimateEventB ? estimateEventA - estimateEventB : estimateEventB - estimateEventA) /
        Number(estimateEventA === 0n ? 1n : estimateEventA);

      return {
        observation: {
          normalized: {
            setSimple: {
              values: [estimateSimpleA.toString(), estimateSimpleB.toString()],
              above21000: estimateSimpleA > 21_000n && estimateSimpleB > 21_000n,
              stabilitySpread: simpleSpread
            },
            emitMultipleEvents: {
              values: [estimateEventA.toString(), estimateEventB.toString()],
              above21000: estimateEventA > 21_000n && estimateEventB > 21_000n,
              stabilitySpread: eventSpread
            },
            revertEstimate: {
              failed: Boolean(revertError),
              errorShape: revertError
                ? {
                    code: revertError.code,
                    hasMessage: typeof revertError.message === "string" && revertError.message.length > 0,
                    hasData: revertError.data !== undefined
                  }
                : null
            }
          }
        },
        requests: [
          {
            method: "eth_estimateGas",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "setSimpleValue"
            }
          },
          {
            method: "eth_estimateGas",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "emitMultipleEvents"
            }
          },
          {
            method: "eth_estimateGas",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "revertWithString"
            }
          }
        ]
      };
    },
    compare: (anvil, rollup) => standardCompare<EstimateGasObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];
      if (!left.setSimple.above21000 || !right.setSimple.above21000) {
        failures.push("setSimple estimates must be above 21,000 gas");
      }
      if (!left.emitMultipleEvents.above21000 || !right.emitMultipleEvents.above21000) {
        failures.push("emitMultipleEvents estimates must be above 21,000 gas");
      }
      if (left.setSimple.stabilitySpread > 0.2 || right.setSimple.stabilitySpread > 0.2) {
        failures.push("setSimple estimates are unstable (>20% spread)");
      }
      if (left.emitMultipleEvents.stabilitySpread > 0.2 || right.emitMultipleEvents.stabilitySpread > 0.2) {
        failures.push("emitMultipleEvents estimates are unstable (>20% spread)");
      }
      if (!left.revertEstimate.failed || !right.revertEstimate.failed) {
        failures.push("reverting call should fail in eth_estimateGas");
      }

      function hugeDiff(a: string, b: string): boolean {
        const leftValue = BigInt(a);
        const rightValue = BigInt(b);
        if (leftValue === 0n || rightValue === 0n) {
          return false;
        }
        return rightValue > leftValue * 5n || rightValue * 5n < leftValue;
      }

      if (hugeDiff(left.setSimple.values[0], right.setSimple.values[0])) {
        failures.push("setSimple estimate differs by more than 5x from anvil baseline");
      }
      if (hugeDiff(left.emitMultipleEvents.values[0], right.emitMultipleEvents.values[0])) {
        failures.push("emitMultipleEvents estimate differs by more than 5x from anvil baseline");
      }

      return failures;
    })
  },
  {
    name: "D.logs_filters_raw",
    library: "raw",
    rpcMethods: ["eth_sendRawTransaction", "eth_getTransactionReceipt", "eth_getLogs"],
    runEndpoint: async (runtime, context) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const iface = new Interface(context.contracts.kitchenSink.abi);
      const indexedValue = 321n;

      const tx = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "emitMultipleEvents",
        [indexedValue, "log-check", "0x11223344"]
      );

      const receipt = await runtime.provider.getTransactionReceipt(tx.txHash);
      if (!receipt) {
        throw new Error("Missing log check receipt");
      }

      const topic0 = keccak256(Buffer.from("ComplexEvent(address,uint256,string,bytes)"));
      const indexedTopic = toBeHex(indexedValue, 32);
      const blockTag = toHexQuantity(receipt.blockNumber);

      const byAddress = await rpc.call("eth_getLogs", [
        {
          address: runtime.deployment.kitchenSink
        }
      ]);

      const byTopic0 = await rpc.call("eth_getLogs", [
        {
          address: runtime.deployment.kitchenSink,
          topics: [topic0],
          fromBlock: blockTag,
          toBlock: blockTag
        }
      ]);

      const byIndexedTopic = await rpc.call("eth_getLogs", [
        {
          address: runtime.deployment.kitchenSink,
          topics: [topic0, null, indexedTopic],
          fromBlock: blockTag,
          toBlock: blockTag
        }
      ]);

      const byRange = await rpc.call("eth_getLogs", [
        {
          address: runtime.deployment.kitchenSink,
          fromBlock: blockTag,
          toBlock: blockTag
        }
      ]);

      const calls = [byAddress, byTopic0, byIndexedTopic, byRange];
      for (const call of calls) {
        if (!call.ok) {
          return {
            observation: {
              error: call.error,
              raw: call.response,
              unsupported: isNotSupportedError(call.error)
            },
            requests: calls.map((entry) => entry.request)
          };
        }
      }

      function decodeLogs(raw: unknown): DecodedHarnessLogEntry[] {
        if (!Array.isArray(raw)) {
          return [];
        }

        const entries: DecodedHarnessLogEntry[] = [];
        for (const item of raw) {
          if (!item || typeof item !== "object") {
            continue;
          }

          const log = item as {
            topics?: unknown;
            data?: unknown;
            address?: unknown;
            transactionHash?: unknown;
          };
          if (!Array.isArray(log.topics) || typeof log.data !== "string" || typeof log.address !== "string") {
            continue;
          }

          const txHash = typeof log.transactionHash === "string" ? log.transactionHash : null;

          try {
            const parsed = iface.parseLog({
              topics: log.topics as string[],
              data: log.data
            });
            entries.push({
              event: parsed?.name ?? "UNKNOWN",
              args: normalizeRuntimeValue(namedArgs(parsed?.args), runtime),
              txHash
            });
          } catch {
            entries.push({
              event: "UNKNOWN",
              txHash,
              topics: log.topics,
              data: log.data
            });
          }
        }

        return entries;
      }

      return {
        observation: {
          normalized: {
            targetTxHash: tx.txHash,
            byAddress: decodeLogs(byAddress.ok ? byAddress.result : null),
            byTopic0: decodeLogs(byTopic0.ok ? byTopic0.result : null),
            byIndexedTopic: decodeLogs(byIndexedTopic.ok ? byIndexedTopic.result : null),
            byRange: decodeLogs(byRange.ok ? byRange.result : null)
          }
        },
        requests: [byAddress.request, byTopic0.request, byIndexedTopic.request, byRange.request]
      };
    },
    compare: (anvil, rollup) => standardCompare<LogsFiltersCheckObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];
      const expectedTxEvents = ["ComplexEvent", "SecondaryEvent"];
      const expectedComplexEvent = ["ComplexEvent"];

      function eventsForTx(entries: DecodedHarnessLogEntry[], txHash: string): string[] {
        const normalizedTxHash = txHash.toLowerCase();
        return entries
          .filter((entry) => typeof entry.txHash === "string" && entry.txHash.toLowerCase() === normalizedTxHash)
          .map((entry) => entry.event);
      }

      function pushIfMismatched(label: string, actual: string[], expected: string[]): void {
        if (actual.length !== expected.length || actual.some((eventName, idx) => eventName !== expected[idx])) {
          failures.push(
            `${label} expected tx events [${expected.join(", ")}], got [${actual.join(", ")}]`
          );
        }
      }

      pushIfMismatched("anvil.byAddress", eventsForTx(left.byAddress, left.targetTxHash), expectedTxEvents);
      pushIfMismatched("rollup.byAddress", eventsForTx(right.byAddress, right.targetTxHash), expectedTxEvents);
      pushIfMismatched("anvil.byRange", eventsForTx(left.byRange, left.targetTxHash), expectedTxEvents);
      pushIfMismatched("rollup.byRange", eventsForTx(right.byRange, right.targetTxHash), expectedTxEvents);

      pushIfMismatched("anvil.byTopic0", eventsForTx(left.byTopic0, left.targetTxHash), expectedComplexEvent);
      pushIfMismatched("rollup.byTopic0", eventsForTx(right.byTopic0, right.targetTxHash), expectedComplexEvent);
      pushIfMismatched(
        "anvil.byIndexedTopic",
        eventsForTx(left.byIndexedTopic, left.targetTxHash),
        expectedComplexEvent
      );
      pushIfMismatched(
        "rollup.byIndexedTopic",
        eventsForTx(right.byIndexedTopic, right.targetTxHash),
        expectedComplexEvent
      );

      return failures;
    })
  },
  {
    name: "E.block_shape_raw",
    library: "raw",
    rpcMethods: ["eth_getBlockByNumber"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const latestHashes = await rpc.call("eth_getBlockByNumber", ["latest", false]);
      const latestFull = await rpc.call("eth_getBlockByNumber", ["latest", true]);

      if (!latestHashes.ok || !latestFull.ok) {
        const error = !latestHashes.ok
          ? latestHashes.error
          : !latestFull.ok
            ? latestFull.error
            : undefined;
        return {
          observation: {
            error,
            raw: {
              latestHashes: latestHashes.response,
              latestFull: latestFull.response
            },
            unsupported: isNotSupportedError(error)
          },
          requests: [latestHashes.request, latestFull.request]
        };
      }

      const shapeHashes = validateBlockShape(latestHashes.result, false);
      const shapeFull = validateBlockShape(latestFull.result, true);

      const blockA = latestHashes.result as Record<string, unknown>;
      const blockB = latestFull.result as Record<string, unknown>;

      return {
        observation: {
          normalized: {
            latestHashesShape: shapeHashes,
            latestFullShape: shapeFull,
            baseFeePresent: {
              latestHashes: blockA.baseFeePerGas !== undefined,
              latestFull: blockB.baseFeePerGas !== undefined
            }
          },
          raw: {
            latestHashes: latestHashes.result,
            latestFull: latestFull.result
          }
        },
        requests: [latestHashes.request, latestFull.request]
      };
    },
    compare: (anvil, rollup) => standardCompare<BlockShapeObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];
      if (!left.latestHashesShape.ok || !left.latestFullShape.ok) {
        failures.push("Anvil baseline block shape validation failed");
      }
      if (!right.latestHashesShape.ok || !right.latestFullShape.ok) {
        failures.push("Rollup block shape validation failed");
      }
      return failures;
    })
  },
  {
    name: "E.chain_fields_raw",
    library: "raw",
    rpcMethods: ["eth_chainId", "net_version", "web3_clientVersion"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const chainId = await rpc.call("eth_chainId", []);
      const netVersion = await rpc.call("net_version", []);
      const clientVersion = await rpc.call("web3_clientVersion", []);

      const requests = [chainId.request, netVersion.request, clientVersion.request];

      if (!chainId.ok) {
        return {
          observation: {
            error: chainId.error,
            raw: {
              chainId: chainId.response,
              netVersion: netVersion.response,
              clientVersion: clientVersion.response
            },
            unsupported: isNotSupportedError(chainId.error)
          },
          requests
        };
      }

      if (!clientVersion.ok) {
        return {
          observation: {
            error: clientVersion.error,
            raw: {
              chainId: chainId.response,
              netVersion: netVersion.response,
              clientVersion: clientVersion.response
            },
            unsupported: isNotSupportedError(clientVersion.error)
          },
          requests
        };
      }

      return {
        observation: {
          normalized: {
            chainId: {
              value: chainId.result,
              validHexQuantity:
                typeof chainId.result === "string" && /^0x[0-9a-fA-F]+$/.test(chainId.result)
            },
            netVersion: netVersion.ok
              ? {
                  supported: true,
                  value: netVersion.result,
                  validDecimalString:
                    typeof netVersion.result === "string" && /^\d+$/.test(netVersion.result)
                }
              : {
                  supported: false,
                  error: netVersion.error
                },
            clientVersion: {
              value: clientVersion.result,
              hasString: typeof clientVersion.result === "string"
            }
          },
          raw: {
            chainId: chainId.response,
            netVersion: netVersion.response,
            clientVersion: clientVersion.response
          }
        },
        requests
      };
    },
    compare: (_anvil, rollup) => {
      if (rollup.unsupported || isNotSupportedError(rollup.error)) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollupError: rollup.error
          }
        };
      }

      if (rollup.error || !rollup.normalized) {
        return {
          outcome: "FAIL",
          diff: {
            rollupError: rollup.error
          }
        };
      }

      const normalized = rollup.normalized as {
        chainId: { validHexQuantity: boolean };
        netVersion: { supported: boolean; validDecimalString?: boolean; error?: RpcErrorShape };
        clientVersion: { hasString: boolean };
      };

      if (!normalized.chainId.validHexQuantity || !normalized.clientVersion.hasString) {
        return {
          outcome: "FAIL",
          diff: {
            reason: "Required chain fields are not shape-conformant",
            details: normalized
          }
        };
      }

      if (!normalized.netVersion.supported) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollupError: normalized.netVersion.error
          }
        };
      }

      if (!normalized.netVersion.validDecimalString) {
        return {
          outcome: "FAIL",
          diff: {
            reason: "net_version returned non-numeric string",
            details: normalized.netVersion
          }
        };
      }

      return { outcome: "PASS" };
    }
  },
  {
    name: "F.rpc_error_conventions_raw",
    library: "raw",
    rpcMethods: ["rpc_nonExistentMethod", "eth_call", "eth_getBlockByNumber"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);

      const missingMethod = await rpc.call("rpc_harness_nonexistent_method", []);
      const badParams = await rpc.call("eth_call", [{}, "latest", "extra-param"]);
      const invalidTag = await rpc.call("eth_getBlockByNumber", ["invalid-tag", false]);

      const calls = [missingMethod, badParams, invalidTag];
      const requests = calls.map((call) => call.request);

      const observation: Record<string, ErrorConventionCase> = {
        missingMethod: toErrorConventionCase(missingMethod),
        badParams: toErrorConventionCase(badParams),
        invalidTag: toErrorConventionCase(invalidTag)
      };

      return {
        observation: {
          normalized: observation,
          raw: {
            missingMethod: missingMethod.response,
            badParams: badParams.response,
            invalidTag: invalidTag.response
          }
        },
        requests
      };
    },
    compare: (anvil, rollup) => {
      if (rollup.error || !anvil.normalized || !rollup.normalized) {
        return {
          outcome: "FAIL",
          diff: {
            anvilError: anvil.error,
            rollupError: rollup.error
          }
        };
      }

      const left = anvil.normalized as Record<string, ErrorConventionCase>;
      const right = rollup.normalized as typeof left;

      const mismatches: Array<{ case: string; reason: string; expected: unknown; actual: unknown }> = [];

      for (const key of ["missingMethod", "badParams", "invalidTag"]) {
        const a = left[key];
        const b = right[key];

        if (!a || !b) {
          mismatches.push({
            case: key,
            reason: "Missing case in normalized observations",
            expected: a,
            actual: b
          });
          continue;
        }

        if (a.ok || b.ok) {
          mismatches.push({
            case: key,
            reason: "Expected JSON-RPC error but received success",
            expected: a,
            actual: b
          });
          continue;
        }

        const anvilCase = a as Exclude<ErrorConventionCase, { ok: true }>;
        const rollupCase = b as Exclude<ErrorConventionCase, { ok: true }>;

        if (anvilCase.code !== rollupCase.code) {
          mismatches.push({
            case: key,
            reason: "Error code mismatch",
            expected: {
              code: anvilCase.code,
              message: anvilCase.message
            },
            actual: {
              code: rollupCase.code,
              message: rollupCase.message
            }
          });
        }

        if (!rollupCase.hasMessage) {
          mismatches.push({
            case: key,
            reason: "Rollup error missing message string",
            expected: {
              hasMessage: anvilCase.hasMessage,
              message: anvilCase.message
            },
            actual: {
              hasMessage: rollupCase.hasMessage,
              message: rollupCase.message
            }
          });
        }

        if (anvilCase.hasData !== rollupCase.hasData) {
          mismatches.push({
            case: key,
            reason: "Error data presence mismatch",
            expected: {
              hasData: anvilCase.hasData,
              data: anvilCase.data
            },
            actual: {
              hasData: rollupCase.hasData,
              data: rollupCase.data
            }
          });
        }
      }

      if (mismatches.length === 0) {
        return { outcome: "PASS" };
      }

      return {
        outcome: "FAIL",
        diff: {
          mismatches,
          expectedBaseline: left,
          actualRollup: right
        }
      };
    }
  },
  {
    name: "G.batch_support_raw",
    library: "raw",
    rpcMethods: ["eth_chainId", "web3_clientVersion", "eth_getBlockByNumber"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const batch = await rpc.batch([
        { method: "eth_chainId", params: [] },
        { method: "web3_clientVersion", params: [] },
        { method: "eth_getBlockByNumber", params: ["latest", false] }
      ]);

      if (!batch.supported) {
        return {
          observation: {
            unsupported: true,
            error: batch.responses[0]?.error,
            raw: batch.raw
          },
          requests: [batch.request]
        };
      }

      const summaries = batch.responses.map((response) => {
        if (!response.ok) {
          return {
            id: response.id,
            method: response.method,
            ok: false,
            code: response.error?.code,
            hasMessage: typeof response.error?.message === "string"
          };
        }

        if (response.method === "eth_chainId") {
          return {
            id: response.id,
            method: response.method,
            ok: true,
            resultShape: typeof response.result === "string" && /^0x[0-9a-fA-F]+$/.test(response.result)
              ? "hex_quantity"
              : "invalid"
          };
        }

        if (response.method === "web3_clientVersion") {
          return {
            id: response.id,
            method: response.method,
            ok: true,
            resultShape: typeof response.result === "string" ? "string" : "invalid"
          };
        }

        if (response.method === "eth_getBlockByNumber") {
          return {
            id: response.id,
            method: response.method,
            ok: true,
            resultShape: validateBlockShape(response.result, false).ok ? "block_object" : "invalid"
          };
        }

        return {
          id: response.id,
          method: response.method,
          ok: true,
          resultShape: "unknown"
        };
      });

      return {
        observation: {
          normalized: {
            supported: true,
            responses: summaries
          },
          raw: batch.raw
        },
        requests: [batch.request]
      };
    },
    compare: (_anvil, rollup) => {
      if (rollup.unsupported || isNotSupportedError(rollup.error)) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollupError: rollup.error
          }
        };
      }

      if (rollup.error || !rollup.normalized) {
        return {
          outcome: "FAIL",
          diff: {
            rollupError: rollup.error
          }
        };
      }

      const normalized = rollup.normalized as {
        supported: boolean;
        responses: Array<{ ok: boolean; method?: string; resultShape?: string }>;
      };

      if (!normalized.supported) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollup
          }
        };
      }

      const failures: string[] = [];
      for (const response of normalized.responses) {
        if (!response.ok) {
          failures.push(`Batch sub-call failed for method ${response.method ?? "unknown"}`);
          continue;
        }

        if (response.resultShape === "invalid") {
          failures.push(`Batch sub-call returned invalid shape for method ${response.method ?? "unknown"}`);
        }
      }

      if (failures.length === 0) {
        return { outcome: "PASS" };
      }

      return {
        outcome: "FAIL",
        diff: {
          failures,
          responses: normalized.responses
        }
      };
    }
  },
  {
    name: "H.block_number_consistency_raw",
    library: "raw",
    rpcMethods: ["eth_blockNumber", "eth_getBlockByNumber"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const blockNumberA = await rpc.call("eth_blockNumber", []);
      const latestBlock = await rpc.call("eth_getBlockByNumber", ["latest", false]);
      const blockNumberB = await rpc.call("eth_blockNumber", []);

      const requests = [blockNumberA.request, latestBlock.request, blockNumberB.request];

      if (!blockNumberA.ok || !latestBlock.ok || !blockNumberB.ok) {
        const failing = !blockNumberA.ok ? blockNumberA : !latestBlock.ok ? latestBlock : blockNumberB;
        if (failing.ok) {
          throw new Error("Expected block number failure branch to contain failed RPC call");
        }

        return {
          observation: {
            error: failing.error,
            unsupported: isNotSupportedError(failing.error),
            raw: {
              blockNumberA: blockNumberA.response,
              latestBlock: latestBlock.response,
              blockNumberB: blockNumberB.response
            }
          },
          requests
        };
      }

      const latest = latestBlock.result as Record<string, unknown>;
      const parsedA = parseHexQuantity(blockNumberA.result);
      const parsedLatest = parseHexQuantity(latest.number);
      const parsedB = parseHexQuantity(blockNumberB.result);

      return {
        observation: {
          normalized: {
            blockNumberA: {
              raw: blockNumberA.result,
              parsed: parsedA !== null ? parsedA.toString() : null,
              validHexQuantity: parsedA !== null
            },
            latestBlockNumber: {
              raw: latest.number,
              parsed: parsedLatest !== null ? parsedLatest.toString() : null,
              validHexQuantity: parsedLatest !== null
            },
            blockNumberB: {
              raw: blockNumberB.result,
              parsed: parsedB !== null ? parsedB.toString() : null,
              validHexQuantity: parsedB !== null
            },
            checks: {
              nonDecreasing: parsedA !== null && parsedB !== null ? parsedB >= parsedA : false,
              latestWithinRange:
                parsedA !== null && parsedLatest !== null && parsedB !== null
                  ? parsedLatest >= parsedA && parsedLatest <= parsedB
                  : false
            }
          },
          raw: {
            blockNumberA: blockNumberA.result,
            latestBlock: latestBlock.result,
            blockNumberB: blockNumberB.result
          }
        },
        requests
      };
    },
    compare: (anvil, rollup) => standardCompare<BlockNumberConsistencyObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];
      if (!left.blockNumberA.validHexQuantity || !left.latestBlockNumber.validHexQuantity || !left.blockNumberB.validHexQuantity) {
        failures.push("Anvil baseline returned invalid block number quantity shape");
      }
      if (!right.blockNumberA.validHexQuantity || !right.latestBlockNumber.validHexQuantity || !right.blockNumberB.validHexQuantity) {
        failures.push("Rollup returned invalid block number quantity shape");
      }
      if (!left.checks.nonDecreasing || !left.checks.latestWithinRange) {
        failures.push("Anvil baseline block number sequence is inconsistent");
      }
      if (!right.checks.nonDecreasing || !right.checks.latestWithinRange) {
        failures.push("Rollup block number sequence is inconsistent");
      }
      return failures;
    })
  },
  {
    name: "I.block_tag_state_reads_raw",
    library: "raw",
    rpcMethods: ["eth_call", "eth_sendRawTransaction", "eth_getTransactionReceipt"],
    runEndpoint: async (runtime, context) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const iface = new Interface(context.contracts.kitchenSink.abi);
      const firstValue = 91_001n;
      const secondValue = 91_002n;

      const firstTx = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "setSimpleValue",
        [firstValue]
      );
      const firstReceipt = await runtime.provider.getTransactionReceipt(firstTx.txHash);

      const secondTx = await sendContractTransactionWithFallback(
        runtime,
        context.contracts.kitchenSink,
        runtime.deployment.kitchenSink,
        "setSimpleValue",
        [secondValue]
      );
      const secondReceipt = await runtime.provider.getTransactionReceipt(secondTx.txHash);

      if (!firstReceipt || !secondReceipt) {
        throw new Error("Missing receipt for block tag state-read check");
      }

      const callData = iface.encodeFunctionData("simpleValue", []);

      async function callAtTag(tag: string): Promise<{
        request: unknown;
        summary:
          | { ok: true; value: string }
          | { ok: false; error?: RpcErrorShape; decodeError?: string };
      }> {
        const result = await rpc.call("eth_call", [{ to: runtime.deployment.kitchenSink, data: callData }, tag]);
        if (!result.ok) {
          return {
            request: result.request,
            summary: {
              ok: false,
              error: result.error
            }
          };
        }

        const decoded = decodeUintFromCallResult(iface, "simpleValue", result.result);
        if (!decoded.ok) {
          return {
            request: result.request,
            summary: {
              ok: false,
              decodeError: decoded.reason
            }
          };
        }

        return {
          request: result.request,
          summary: {
            ok: true,
            value: decoded.value
          }
        };
      }

      const firstTag = toHexQuantity(firstReceipt.blockNumber);
      const secondTag = toHexQuantity(secondReceipt.blockNumber);
      const byFirst = await callAtTag(firstTag);
      const bySecond = await callAtTag(secondTag);
      const byLatest = await callAtTag("latest");
      const byPending = await callAtTag("pending");

      const historicalComparable = firstReceipt.blockNumber < secondReceipt.blockNumber;

      const atFirstMatches = byFirst.summary.ok && byFirst.summary.value === firstValue.toString();
      const atSecondMatches = bySecond.summary.ok && bySecond.summary.value === secondValue.toString();
      const latestMatches = byLatest.summary.ok && byLatest.summary.value === secondValue.toString();

      return {
        observation: {
          normalized: {
            expected: {
              firstValue: firstValue.toString(),
              secondValue: secondValue.toString()
            },
            blocks: {
              firstTag,
              secondTag,
              firstBlockNumber: firstReceipt.blockNumber,
              secondBlockNumber: secondReceipt.blockNumber,
              historicalComparable,
              sameBlock: firstReceipt.blockNumber === secondReceipt.blockNumber
            },
            reads: {
              atFirst: byFirst.summary,
              atSecond: bySecond.summary,
              latest: byLatest.summary,
              pending: byPending.summary
            },
            checks: {
              atSecondMatchesExpected: atSecondMatches,
              latestMatchesExpected: latestMatches,
              atFirstMatchesExpectedWhenComparable: historicalComparable ? atFirstMatches : null,
              pendingHasMessageIfErrored:
                byPending.summary.ok ||
                (typeof byPending.summary.error?.message === "string" && byPending.summary.error.message.length > 0)
            }
          }
        },
        requests: [
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "setSimpleValue",
              args: [firstValue.toString()]
            }
          },
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "setSimpleValue",
              args: [secondValue.toString()]
            }
          },
          byFirst.request,
          bySecond.request,
          byLatest.request,
          byPending.request
        ]
      };
    },
    compare: (anvil, rollup) => standardCompare<BlockTagStateReadsObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];

      if (!left.checks.atSecondMatchesExpected || !left.checks.latestMatchesExpected) {
        failures.push("Anvil baseline failed latest/second block-tag expectations");
      }
      if (left.blocks.historicalComparable && left.checks.atFirstMatchesExpectedWhenComparable !== true) {
        failures.push("Anvil baseline failed historical block-tag expectation");
      }

      if (!right.checks.atSecondMatchesExpected) {
        failures.push("Rollup eth_call at second block tag returned unexpected value");
      }
      if (!right.checks.latestMatchesExpected) {
        failures.push("Rollup eth_call at latest returned unexpected value");
      }
      if (right.blocks.historicalComparable && right.checks.atFirstMatchesExpectedWhenComparable !== true) {
        failures.push("Rollup eth_call at first block tag did not return historical value");
      }
      if (!right.checks.pendingHasMessageIfErrored) {
        failures.push("Rollup pending-tag eth_call errored without message string");
      }

      if (failures.length > 0) return failures;

      const notes: string[] = [];
      if (right.blocks.sameBlock) {
        notes.push("Rollup sealed both writes in one block; historical tag assertion was skipped.");
      }
      if (left.blocks.sameBlock) {
        notes.push("Anvil sealed both writes in one block; historical baseline assertion was skipped.");
      }

      if (notes.length > 0) return { outcome: "PASS" as const, notes };
      return failures;
    })
  },
  {
    name: "J.pending_to_sealed_tx_raw",
    library: "raw",
    rpcMethods: ["eth_sendRawTransaction", "eth_getTransactionByHash", "eth_getTransactionReceipt"],
    runEndpoint: async (runtime, context) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);
      const kitchen = new Contract(runtime.deployment.kitchenSink, context.contracts.kitchenSink.abi, runtime.wallet);
      const calldata = kitchen.interface.encodeFunctionData("setSimpleValue", [92_001n]);

      const nonce = await runtime.provider.getTransactionCount(runtime.wallet.address, "pending");
      const feeData = await runtime.provider.getFeeData();
      const maxPriorityFeePerGas = feeData.maxPriorityFeePerGas ?? feeData.gasPrice ?? 1_000_000_000n;
      const maxFeePerGas = feeData.maxFeePerGas ?? maxPriorityFeePerGas * 2n;

      let txTypeUsed: "eip1559" | "legacy" = "eip1559";
      let txHash: string;

      try {
        const response = await runtime.wallet.sendTransaction({
          to: runtime.deployment.kitchenSink,
          data: calldata,
          nonce,
          gasLimit: 300_000n,
          chainId: runtime.chainId,
          type: 2,
          maxFeePerGas,
          maxPriorityFeePerGas
        });
        txHash = response.hash;
      } catch (error) {
        if (!isLikely1559Unsupported(error)) {
          throw error;
        }

        const gasPrice = feeData.gasPrice ?? 1_000_000_000n;
        txTypeUsed = "legacy";

        const response = await runtime.wallet.sendTransaction({
          to: runtime.deployment.kitchenSink,
          data: calldata,
          nonce,
          gasLimit: 300_000n,
          chainId: runtime.chainId,
          type: 0,
          gasPrice
        });
        txHash = response.hash;
      }

      const txImmediate = await rpc.call("eth_getTransactionByHash", [txHash]);
      const receiptInitial = await rpc.call("eth_getTransactionReceipt", [txHash]);
      const receiptRequests: unknown[] = [receiptInitial.request];

      let receiptFinal = receiptInitial;
      for (let i = 0; i < 80; i += 1) {
        if (!receiptFinal.ok || receiptFinal.result !== null) {
          break;
        }

        await delay(250);
        receiptFinal = await rpc.call("eth_getTransactionReceipt", [txHash]);
        receiptRequests.push(receiptFinal.request);
      }

      const txFinal = await rpc.call("eth_getTransactionByHash", [txHash]);

      if (!txImmediate.ok || !receiptInitial.ok || !receiptFinal.ok || !txFinal.ok) {
        const failing = !txImmediate.ok
          ? txImmediate
          : !receiptInitial.ok
            ? receiptInitial
            : !receiptFinal.ok
              ? receiptFinal
              : txFinal;

        if (failing.ok) {
          throw new Error("Expected tx/receipt failure branch to contain failed RPC call");
        }

        return {
          observation: {
            error: failing.error,
            unsupported: isNotSupportedError(failing.error),
            raw: {
              txImmediate: txImmediate.response,
              receiptInitial: receiptInitial.response,
              receiptFinal: receiptFinal.response,
              txFinal: txFinal.response
            }
          },
          requests: [txImmediate.request, ...receiptRequests, txFinal.request]
        };
      }

      const txImmediateObj =
        txImmediate.result && typeof txImmediate.result === "object"
          ? (txImmediate.result as Record<string, unknown>)
          : null;
      const txFinalObj =
        txFinal.result && typeof txFinal.result === "object"
          ? (txFinal.result as Record<string, unknown>)
          : null;
      const receiptInitialObj =
        receiptInitial.result && typeof receiptInitial.result === "object"
          ? (receiptInitial.result as Record<string, unknown>)
          : null;
      const receiptFinalObj =
        receiptFinal.result && typeof receiptFinal.result === "object"
          ? (receiptFinal.result as Record<string, unknown>)
          : null;

      const receiptStatus = receiptFinalObj ? parseHexQuantity(receiptFinalObj.status) : null;

      return {
        observation: {
          normalized: {
            txTypeUsed,
            txHash,
            txLookupImmediate: {
              found: txImmediateObj !== null,
              hasBlockHash: txImmediateObj !== null && typeof txImmediateObj.blockHash === "string",
              hasBlockNumber: txImmediateObj !== null && typeof txImmediateObj.blockNumber === "string"
            },
            txLookupFinal: {
              found: txFinalObj !== null,
              hasBlockHash: txFinalObj !== null && typeof txFinalObj.blockHash === "string",
              hasBlockNumber: txFinalObj !== null && typeof txFinalObj.blockNumber === "string"
            },
            receiptImmediateWasNull: receiptInitial.result === null,
            receiptInitialFound: receiptInitialObj !== null,
            receiptFinal: {
              found: receiptFinalObj !== null,
              status: receiptStatus !== null ? receiptStatus.toString() : null,
              hasBlockHash: receiptFinalObj !== null && typeof receiptFinalObj.blockHash === "string",
              hasBlockNumber: receiptFinalObj !== null && typeof receiptFinalObj.blockNumber === "string"
            },
            checks: {
              txFoundEventually: txFinalObj !== null,
              receiptFoundEventually: receiptFinalObj !== null,
              receiptHasStatus: receiptStatus !== null,
              receiptHashMatches:
                receiptFinalObj !== null && typeof receiptFinalObj.transactionHash === "string"
                  ? receiptFinalObj.transactionHash.toLowerCase() === txHash.toLowerCase()
                  : false
            }
          },
          raw: {
            txImmediate: txImmediate.result,
            txFinal: txFinal.result,
            receiptInitial: receiptInitial.result,
            receiptFinal: receiptFinal.result
          }
        },
        requests: [
          {
            method: "eth_sendRawTransaction",
            payload: {
              to: runtime.deployment.kitchenSink,
              function: "setSimpleValue",
              args: ["92001"],
              nonce,
              txTypeUsed
            }
          },
          txImmediate.request,
          ...receiptRequests,
          txFinal.request
        ]
      };
    },
    compare: (anvil, rollup) => standardCompare<PendingToSealedObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];
      const baselineChecks = [
        left.checks.txFoundEventually,
        left.checks.receiptFoundEventually,
        left.checks.receiptHasStatus,
        left.checks.receiptHashMatches
      ];
      if (baselineChecks.some((entry) => !entry)) {
        failures.push("Anvil baseline tx/receipt transition assertions failed");
      }

      if (!right.checks.txFoundEventually) {
        failures.push("Rollup tx lookup never returned transaction object");
      }
      if (!right.checks.receiptFoundEventually) {
        failures.push("Rollup receipt lookup never returned a mined receipt");
      }
      if (!right.checks.receiptHasStatus) {
        failures.push("Rollup receipt missing status");
      }
      if (!right.checks.receiptHashMatches) {
        failures.push("Rollup receipt transactionHash does not match sent tx hash");
      }

      return failures;
    })
  },
  {
    name: "K.block_lookup_and_tx_count_raw",
    library: "raw",
    rpcMethods: [
      "eth_getBlockByHash",
      "eth_getBlockByNumber",
      "eth_getBlockTransactionCountByNumber",
      "eth_getBlockTransactionCountByHash"
    ],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);

      const latestByNumber = await rpc.call("eth_getBlockByNumber", ["latest", false]);
      if (!latestByNumber.ok) {
        return {
          observation: {
            error: latestByNumber.error,
            unsupported: isNotSupportedError(latestByNumber.error),
            raw: latestByNumber.response
          },
          requests: [latestByNumber.request]
        };
      }

      const latestObj = latestByNumber.result as Record<string, unknown>;
      const latestHash = typeof latestObj.hash === "string" ? latestObj.hash : null;
      if (!latestHash) {
        return {
          observation: {
            error: {
              code: null,
              message: "Latest block did not include hash"
            },
            raw: latestByNumber.result
          },
          requests: [latestByNumber.request]
        };
      }

      const byHashShort = await rpc.call("eth_getBlockByHash", [latestHash, false]);
      const byHashFull = await rpc.call("eth_getBlockByHash", [latestHash, true]);
      const txCountByNumber = await rpc.call("eth_getBlockTransactionCountByNumber", ["latest"]);
      const txCountByHash = await rpc.call("eth_getBlockTransactionCountByHash", [latestHash]);
      const invalidHash = await rpc.call("eth_getBlockByHash", ["0x0000000000000000000000000000000000000000000000000000000000000000", false]);

      const requests = [
        latestByNumber.request,
        byHashShort.request,
        byHashFull.request,
        txCountByNumber.request,
        txCountByHash.request,
        invalidHash.request
      ];

      if (!byHashShort.ok || !byHashFull.ok) {
        const failing = !byHashShort.ok ? byHashShort : byHashFull;
        if (failing.ok) {
          throw new Error("Expected block lookup failure branch to contain failed RPC call");
        }

        return {
          observation: {
            error: failing.error,
            unsupported: isNotSupportedError(failing.error),
            raw: {
              latestByNumber: latestByNumber.result,
              byHashShort: byHashShort.response,
              byHashFull: byHashFull.response
            }
          },
          requests
        };
      }

      const txCountUnsupported =
        (!txCountByNumber.ok && isNotSupportedError(txCountByNumber.error)) ||
        (!txCountByHash.ok && isNotSupportedError(txCountByHash.error));

      if ((!txCountByNumber.ok || !txCountByHash.ok) && !txCountUnsupported) {
        const failing = !txCountByNumber.ok ? txCountByNumber : txCountByHash;
        if (failing.ok) {
          throw new Error("Expected tx-count failure branch to contain failed RPC call");
        }

        return {
          observation: {
            error: failing.error,
            raw: {
              txCountByNumber: txCountByNumber.response,
              txCountByHash: txCountByHash.response
            }
          },
          requests
        };
      }

      const latestShortShape = validateBlockShape(latestByNumber.result, false);
      const hashShortShape = validateBlockShape(byHashShort.result, false);
      const hashFullShape = validateBlockShape(byHashFull.result, true);

      const byHashShortObj = byHashShort.result as Record<string, unknown>;
      const byHashFullObj = byHashFull.result as Record<string, unknown>;

      const latestTxs = Array.isArray(latestObj.transactions) ? latestObj.transactions : null;
      const txCountByNumberParsed = txCountByNumber.ok ? parseHexQuantity(txCountByNumber.result) : null;
      const txCountByHashParsed = txCountByHash.ok ? parseHexQuantity(txCountByHash.result) : null;

      return {
        observation: {
          normalized: {
            shapes: {
              latestByNumber: latestShortShape,
              byHashShort: hashShortShape,
              byHashFull: hashFullShape
            },
            linkage: {
              hashMatchesByHash: byHashShortObj.hash === latestObj.hash,
              numberMatchesByHash: byHashShortObj.number === latestObj.number,
              hashMatchesFull: byHashFullObj.hash === latestObj.hash
            },
            txCount: {
              supported: !txCountUnsupported,
              byNumberRaw: txCountByNumber.ok ? txCountByNumber.result : null,
              byHashRaw: txCountByHash.ok ? txCountByHash.result : null,
              byNumberParsed: txCountByNumberParsed !== null ? txCountByNumberParsed.toString() : null,
              byHashParsed: txCountByHashParsed !== null ? txCountByHashParsed.toString() : null,
              validHex:
                txCountUnsupported ||
                (txCountByNumber.ok && txCountByHash.ok && txCountByNumberParsed !== null && txCountByHashParsed !== null),
              equalsEachOther:
                txCountUnsupported ||
                (txCountByNumberParsed !== null && txCountByHashParsed !== null
                  ? txCountByNumberParsed === txCountByHashParsed
                  : false),
              matchesLatestArrayLength:
                txCountUnsupported ||
                (latestTxs !== null && txCountByHashParsed !== null
                  ? txCountByHashParsed === BigInt(latestTxs.length)
                  : false),
              unsupportedError: txCountUnsupported
                ? (!txCountByNumber.ok ? txCountByNumber.error : !txCountByHash.ok ? txCountByHash.error : undefined)
                : undefined
            },
            invalidHashBehavior: {
              returnedNull: invalidHash.ok ? invalidHash.result === null : false,
              error: invalidHash.ok ? null : invalidHash.error
            }
          },
          raw: {
            latestByNumber: latestByNumber.result,
            byHashShort: byHashShort.result,
            byHashFull: byHashFull.result,
            txCountByNumber: txCountByNumber.ok ? txCountByNumber.result : txCountByNumber.response,
            txCountByHash: txCountByHash.ok ? txCountByHash.result : txCountByHash.response,
            invalidHash: invalidHash.ok ? invalidHash.result : invalidHash.response
          }
        },
        requests
      };
    },
    compare: (anvil, rollup) => {
      if (rollup.error || !anvil.normalized || !rollup.normalized) {
        return {
          outcome: "FAIL",
          diff: {
            anvilError: anvil.error,
            rollupError: rollup.error
          }
        };
      }

      const left = anvil.normalized as BlockLookupObservation;
      const right = rollup.normalized as BlockLookupObservation;

      if (!right.txCount.supported && isNotSupportedError(right.txCount.unsupportedError)) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollupError: right.txCount.unsupportedError
          }
        };
      }

      const failures: string[] = [];

      if (!left.shapes.latestByNumber.ok || !left.shapes.byHashShort.ok || !left.shapes.byHashFull.ok) {
        failures.push("Anvil baseline block shape failed for block-by-hash/number lookups");
      }
      if (!right.shapes.latestByNumber.ok || !right.shapes.byHashShort.ok || !right.shapes.byHashFull.ok) {
        failures.push("Rollup block shape failed for block-by-hash/number lookups");
      }

      if (!right.linkage.hashMatchesByHash || !right.linkage.numberMatchesByHash || !right.linkage.hashMatchesFull) {
        failures.push("Rollup block-by-hash lookups are inconsistent with latest block");
      }

      if (!right.txCount.supported) {
        failures.push("Rollup does not support block transaction count methods");
      } else {
        if (!right.txCount.validHex) {
          failures.push("Rollup block transaction count methods returned invalid hex quantity");
        }
        if (!right.txCount.equalsEachOther) {
          failures.push("Rollup block transaction count by hash/number mismatch");
        }
        if (!right.txCount.matchesLatestArrayLength) {
          failures.push("Rollup tx count does not match latest block transactions length");
        }
      }

      if (!right.invalidHashBehavior.returnedNull && right.invalidHashBehavior.error === null) {
        failures.push("Rollup invalid block hash lookup neither returned null nor error");
      }

      if (failures.length === 0) {
        return { outcome: "PASS" };
      }

      return {
        outcome: "FAIL",
        diff: {
          failures,
          anvil: left,
          rollup: right
        }
      };
    }
  },
  {
    name: "L.fee_history_raw",
    library: "raw",
    rpcMethods: ["eth_feeHistory"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);

      const history = await rpc.call("eth_feeHistory", [toHexQuantity(5), "latest", [10, 50, 90]]);
      const invalidTag = await rpc.call("eth_feeHistory", [toHexQuantity(2), "invalid-tag", [50]]);

      const requests = [history.request, invalidTag.request];

      if (!history.ok) {
        return {
          observation: {
            error: history.error,
            unsupported: isNotSupportedError(history.error),
            raw: {
              history: history.response,
              invalidTag: invalidTag.response
            }
          },
          requests
        };
      }

      const feeHistory = history.result as Record<string, unknown>;
      const baseFees = Array.isArray(feeHistory.baseFeePerGas) ? feeHistory.baseFeePerGas : [];
      const gasUsedRatio = Array.isArray(feeHistory.gasUsedRatio) ? feeHistory.gasUsedRatio : [];
      const reward = feeHistory.reward;

      const rewardShapeValid =
        reward === undefined ||
        (Array.isArray(reward) &&
          reward.length === 5 &&
          reward.every(
            (entry) =>
              Array.isArray(entry) &&
              entry.length === 3 &&
              entry.every((item) => parseHexQuantity(item) !== null)
          ));

      return {
        observation: {
          normalized: {
            shape: {
              oldestBlockHex: parseHexQuantity(feeHistory.oldestBlock) !== null,
              baseFeePerGasLength: baseFees.length,
              baseFeePerGasAllHex: baseFees.every((item) => parseHexQuantity(item) !== null),
              gasUsedRatioLength: gasUsedRatio.length,
              gasUsedRatioAllNumbers: gasUsedRatio.every((item) => typeof item === "number"),
              rewardShapeValid
            },
            invalidTag: invalidTag.ok
              ? { ok: true, result: invalidTag.result }
              : {
                  ok: false,
                  code: invalidTag.error.code,
                  message: invalidTag.error.message,
                  hasMessage: invalidTag.error.message.length > 0
                }
          },
          raw: {
            history: history.result,
            invalidTag: invalidTag.ok ? invalidTag.result : invalidTag.response
          }
        },
        requests
      };
    },
    compare: (anvil, rollup) => standardCompare<FeeHistoryObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];

      const leftShapeOk =
        left.shape.oldestBlockHex &&
        left.shape.baseFeePerGasLength === 6 &&
        left.shape.baseFeePerGasAllHex &&
        left.shape.gasUsedRatioLength === 5 &&
        left.shape.gasUsedRatioAllNumbers &&
        left.shape.rewardShapeValid;
      if (!leftShapeOk) {
        failures.push("Anvil baseline feeHistory shape validation failed");
      }

      const rightShapeOk =
        right.shape.oldestBlockHex &&
        right.shape.baseFeePerGasLength === 6 &&
        right.shape.baseFeePerGasAllHex &&
        right.shape.gasUsedRatioLength === 5 &&
        right.shape.gasUsedRatioAllNumbers &&
        right.shape.rewardShapeValid;
      if (!rightShapeOk) {
        failures.push("Rollup feeHistory shape validation failed");
      }

      if (right.invalidTag.ok) {
        failures.push("Rollup feeHistory invalid-tag call unexpectedly succeeded");
      } else if (!right.invalidTag.hasMessage) {
        failures.push("Rollup feeHistory invalid-tag error missing message");
      }

      return failures;
    })
  },
  {
    name: "M.block_receipts_raw",
    library: "raw",
    rpcMethods: ["eth_getBlockByNumber", "eth_getBlockReceipts"],
    runEndpoint: async (runtime) => {
      const rpc = new JsonRpcClient(runtime.rpcUrl);

      const latest = await rpc.call("eth_getBlockByNumber", ["latest", false]);
      if (!latest.ok) {
        return {
          observation: {
            error: latest.error,
            unsupported: isNotSupportedError(latest.error),
            raw: latest.response
          },
          requests: [latest.request]
        };
      }

      const latestBlock = latest.result as Record<string, unknown>;
      const latestHash = typeof latestBlock.hash === "string" ? latestBlock.hash : null;
      const txHashes = Array.isArray(latestBlock.transactions) ? latestBlock.transactions : [];

      if (!latestHash) {
        return {
          observation: {
            error: {
              code: null,
              message: "Latest block hash missing for block receipts check"
            },
            raw: latest.result
          },
          requests: [latest.request]
        };
      }

      const receipts = await rpc.call("eth_getBlockReceipts", [latestHash]);
      const requests = [latest.request, receipts.request];

      if (!receipts.ok) {
        return {
          observation: {
            error: receipts.error,
            unsupported: isNotSupportedError(receipts.error),
            raw: {
              latest: latest.result,
              receipts: receipts.response
            }
          },
          requests
        };
      }

      const receiptEntries = Array.isArray(receipts.result) ? receipts.result : null;

      const shapeOk =
        receiptEntries?.every((entry) => {
          if (!entry || typeof entry !== "object") {
            return false;
          }
          const receipt = entry as Record<string, unknown>;
          return (
            typeof receipt.transactionHash === "string" &&
            (receipt.status === undefined || parseHexQuantity(receipt.status) !== null) &&
            Array.isArray(receipt.logs)
          );
        });

      return {
        observation: {
          normalized: {
            expectedTxCount: txHashes.length,
            receiptCount: receiptEntries ? receiptEntries.length : null,
            shapeOk,
            countMatchesBlock: receiptEntries ? receiptEntries.length === txHashes.length : false
          },
          raw: {
            latest: latest.result,
            receipts: receipts.result
          }
        },
        requests
      };
    },
    compare: (anvil, rollup) => standardCompare<BlockReceiptsObservation>(anvil, rollup, (left, right) => {
      const failures: string[] = [];
      if (!left.shapeOk || !left.countMatchesBlock) {
        failures.push("Anvil baseline block receipts shape/count validation failed");
      }
      if (!right.shapeOk) {
        failures.push("Rollup block receipts returned invalid receipt shape");
      }
      if (!right.countMatchesBlock) {
        failures.push("Rollup block receipts count does not match block transaction count");
      }
      return failures;
    })
  }
];

export async function runChecks(context: HarnessContext): Promise<CheckResult[]> {
  const results: CheckResult[] = [];

  for (const check of checks) {
    results.push(await runCheck(check, context));
  }

  return results;
}
