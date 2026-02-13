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
  summarizeErrorShape,
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
    compare: (anvil, rollup) => {
      if (rollup.unsupported || isNotSupportedError(rollup.error)) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollupError: rollup.error
          }
        };
      }

      if (anvil.error || rollup.error || !anvil.normalized || !rollup.normalized) {
        return {
          outcome: "FAIL",
          diff: {
            anvilError: anvil.error,
            rollupError: rollup.error
          }
        };
      }

      const left = anvil.normalized as {
        setSimple: { values: string[]; above21000: boolean; stabilitySpread: number };
        emitMultipleEvents: { values: string[]; above21000: boolean; stabilitySpread: number };
        revertEstimate: { failed: boolean };
      };
      const right = rollup.normalized as typeof left;

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
          topics: [topic0]
        }
      ]);

      const byIndexedTopic = await rpc.call("eth_getLogs", [
        {
          address: runtime.deployment.kitchenSink,
          topics: [topic0, null, indexedTopic]
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

      function decodeLogs(raw: unknown): unknown[] {
        if (!Array.isArray(raw)) {
          return [];
        }

        const entries: unknown[] = [];
        for (const item of raw) {
          if (!item || typeof item !== "object") {
            continue;
          }

          const log = item as { topics?: unknown; data?: unknown; address?: unknown };
          if (!Array.isArray(log.topics) || typeof log.data !== "string" || typeof log.address !== "string") {
            continue;
          }

          try {
            const parsed = iface.parseLog({
              topics: log.topics as string[],
              data: log.data
            });
            entries.push({
              event: parsed?.name,
              args: normalizeRuntimeValue(namedArgs(parsed?.args), runtime)
            });
          } catch {
            entries.push({
              event: "UNKNOWN",
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
            byAddress: decodeLogs(byAddress.ok ? byAddress.result : null),
            byTopic0: decodeLogs(byTopic0.ok ? byTopic0.result : null),
            byIndexedTopic: decodeLogs(byIndexedTopic.ok ? byIndexedTopic.result : null),
            byRange: decodeLogs(byRange.ok ? byRange.result : null)
          }
        },
        requests: [byAddress.request, byTopic0.request, byIndexedTopic.request, byRange.request]
      };
    }
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
    compare: (anvil, rollup) => {
      if (rollup.unsupported || isNotSupportedError(rollup.error)) {
        return {
          outcome: "NOT_SUPPORTED",
          diff: {
            rollupError: rollup.error
          }
        };
      }

      if (anvil.error || rollup.error || !anvil.normalized || !rollup.normalized) {
        return {
          outcome: "FAIL",
          diff: {
            anvilError: anvil.error,
            rollupError: rollup.error
          }
        };
      }

      const left = anvil.normalized as {
        latestHashesShape: { ok: boolean; issues: string[] };
        latestFullShape: { ok: boolean; issues: string[] };
      };
      const right = rollup.normalized as typeof left;

      const failures: string[] = [];
      if (!left.latestHashesShape.ok || !left.latestFullShape.ok) {
        failures.push("Anvil baseline block shape validation failed");
      }
      if (!right.latestHashesShape.ok || !right.latestFullShape.ok) {
        failures.push("Rollup block shape validation failed");
      }

      if (failures.length === 0) {
        return { outcome: "PASS" };
      }

      return {
        outcome: "FAIL",
        diff: {
          failures,
          anvil,
          rollup
        }
      };
    }
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

      const observation = {
        missingMethod: missingMethod.ok
          ? { ok: true }
          : { ok: false, ...summarizeErrorShape(missingMethod.error), raw: missingMethod.error },
        badParams: badParams.ok
          ? { ok: true }
          : { ok: false, ...summarizeErrorShape(badParams.error), raw: badParams.error },
        invalidTag: invalidTag.ok
          ? { ok: true }
          : { ok: false, ...summarizeErrorShape(invalidTag.error), raw: invalidTag.error }
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

      const left = anvil.normalized as Record<string, { ok: boolean; code?: number | null; hasMessage?: boolean; hasData?: boolean }>;
      const right = rollup.normalized as typeof left;

      const mismatches: Array<{ case: string; reason: string; anvil: unknown; rollup: unknown }> = [];

      for (const key of ["missingMethod", "badParams", "invalidTag"]) {
        const a = left[key];
        const b = right[key];

        if (!a || !b) {
          mismatches.push({
            case: key,
            reason: "Missing case in normalized observations",
            anvil: a,
            rollup: b
          });
          continue;
        }

        if (a.ok || b.ok) {
          mismatches.push({
            case: key,
            reason: "Expected JSON-RPC error but received success",
            anvil: a,
            rollup: b
          });
          continue;
        }

        if (a.code !== b.code) {
          mismatches.push({
            case: key,
            reason: "Error code mismatch",
            anvil: a.code,
            rollup: b.code
          });
        }

        if (!b.hasMessage) {
          mismatches.push({
            case: key,
            reason: "Rollup error missing message string",
            anvil: a.hasMessage,
            rollup: b.hasMessage
          });
        }

        if (a.hasData !== b.hasData) {
          mismatches.push({
            case: key,
            reason: "Error data presence mismatch",
            anvil: a.hasData,
            rollup: b.hasData
          });
        }
      }

      if (mismatches.length === 0) {
        return { outcome: "PASS" };
      }

      return {
        outcome: "FAIL",
        diff: mismatches
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
  }
];

export async function runChecks(context: HarnessContext): Promise<CheckResult[]> {
  const results: CheckResult[] = [];

  for (const check of checks) {
    results.push(await runCheck(check, context));
  }

  return results;
}
