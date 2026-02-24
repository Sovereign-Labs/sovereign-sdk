import { readFile } from "node:fs/promises";
import path from "node:path";
import { Contract, ContractFactory, JsonRpcProvider, Wallet } from "ethers";
import { createPublicClient, http } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import type {
  CompiledContracts,
  ContractArtifact,
  DeploymentState,
  EndpointConfig,
  EndpointRuntime,
  RpcErrorShape
} from "./types";

export interface SendTransactionResult {
  txHash: string;
  receipt: {
    status: number;
    logsLength: number;
    effectiveGasPricePresent: boolean;
    contractAddress: string | null;
  };
  txTypeUsed: "eip1559" | "legacy";
}

const DEPLOY_TX_GAS_LIMIT = 8_000_000n;
const CONTRACT_TX_GAS_LIMIT = 3_000_000n;
const VALUE_TX_GAS_LIMIT = 21_000n;

function parseArtifact(json: string): ContractArtifact {
  const artifact = JSON.parse(json) as {
    abi?: unknown;
    bytecode?: unknown;
  };

  if (!artifact.abi || !artifact.bytecode || typeof artifact.bytecode !== "string") {
    throw new Error("Invalid contract artifact format");
  }

  return {
    abi: artifact.abi as ContractArtifact["abi"],
    bytecode: artifact.bytecode
  };
}

async function readArtifact(appRoot: string, contractName: string): Promise<ContractArtifact> {
  const artifactPath = path.join(appRoot, "artifacts", "contracts", "KitchenSink.sol", `${contractName}.json`);
  const contents = await readFile(artifactPath, "utf-8");
  return parseArtifact(contents);
}

export async function loadCompiledContracts(appRoot: string): Promise<CompiledContracts> {
  return {
    kitchenSink: await readArtifact(appRoot, "KitchenSink"),
    delegateTarget: await readArtifact(appRoot, "DelegateTarget"),
    callReceiver: await readArtifact(appRoot, "CallReceiver"),
    create2Child: await readArtifact(appRoot, "Create2Child")
  };
}

function errorMessage(error: unknown): string {
  if (!error) {
    return "Unknown error";
  }

  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "object") {
    const maybe = error as { shortMessage?: unknown; message?: unknown };
    if (typeof maybe.shortMessage === "string") {
      return maybe.shortMessage;
    }
    if (typeof maybe.message === "string") {
      return maybe.message;
    }
  }

  return String(error);
}

function isLikely1559CompatibilityIssue(error: unknown): boolean {
  const message = errorMessage(error).toLowerCase();
  return (
    message.includes("1559") ||
    message.includes("transaction type") ||
    message.includes("unsupported transaction") ||
    message.includes("maxfeepergas")
  );
}

function hasEffectiveGasPrice(
  receipt: { effectiveGasPrice?: bigint | null; gasPrice?: bigint | null }
): boolean {
  return receipt.effectiveGasPrice !== undefined || receipt.gasPrice !== undefined;
}

function isNonceConflictMessage(message: string): boolean {
  const normalized = message.toLowerCase();
  return (
    normalized.includes("nonce too low") ||
    normalized.includes("nonce has already been used") ||
    normalized.includes("nonce is too low") ||
    normalized.includes("replacement transaction underpriced") ||
    normalized.includes("already known")
  );
}

function isNonceConflictError(error: unknown): boolean {
  if (isNonceConflictMessage(errorMessage(error))) {
    return true;
  }

  if (error && typeof error === "object") {
    const maybe = error as {
      code?: unknown;
      info?: { error?: { code?: unknown; message?: unknown } };
      error?: { code?: unknown; message?: unknown };
    };

    if (maybe.code === "NONCE_EXPIRED") {
      return true;
    }

    if (
      maybe.info &&
      typeof maybe.info === "object" &&
      maybe.info.error &&
      typeof maybe.info.error === "object" &&
      maybe.info.error.code === -32003 &&
      typeof maybe.info.error.message === "string" &&
      isNonceConflictMessage(maybe.info.error.message)
    ) {
      return true;
    }

    if (
      maybe.error &&
      typeof maybe.error === "object" &&
      maybe.error.code === -32003 &&
      typeof maybe.error.message === "string" &&
      isNonceConflictMessage(maybe.error.message)
    ) {
      return true;
    }
  }

  return false;
}

function isTransientOverloadError(error: unknown): boolean {
  const message = errorMessage(error).toLowerCase();
  return (
    message.includes("temporarily overloaded") ||
    message.includes("no finalized slots available") ||
    (message.includes("503") && message.includes("service unavailable"))
  );
}

async function sendWithNonceRecovery(
  wallet: Wallet,
  request: {
    to?: string;
    data?: string;
    value?: bigint;
    type: 0 | 2;
    gasLimit?: bigint;
    gasPrice?: bigint;
    maxFeePerGas?: bigint;
    maxPriorityFeePerGas?: bigint;
  }
) {
  if (!wallet.provider) {
    throw new Error("Wallet provider is not attached");
  }

  let forcedNonce: number | undefined;
  let lastError: unknown;

  for (let attempt = 0; attempt < 6; attempt += 1) {
    try {
      const txResponse = await wallet.sendTransaction({
        ...request,
        ...(forcedNonce !== undefined ? { nonce: forcedNonce } : {})
      });

      const receipt = await txResponse.wait();
      if (!receipt) {
        throw new Error("Transaction receipt missing");
      }

      return { txResponse, receipt };
    } catch (error) {
      lastError = error;

      if (isTransientOverloadError(error)) {
        const backoffMs = Math.min(1000 * 2 ** attempt, 15_000);
        console.warn(
          `Sequencer overloaded (attempt ${attempt + 1}/6), retrying in ${backoffMs}ms...`
        );
        await new Promise((resolve) => setTimeout(resolve, backoffMs));
        continue;
      }

      if (!isNonceConflictError(error)) {
        throw error;
      }

      const [pendingNonce, latestNonce] = await Promise.all([
        wallet.provider.getTransactionCount(wallet.address, "pending"),
        wallet.provider.getTransactionCount(wallet.address, "latest")
      ]);
      const suggested = Math.max(pendingNonce, latestNonce);
      forcedNonce =
        forcedNonce !== undefined && suggested <= forcedNonce ? forcedNonce + 1 : suggested;
    }
  }

  throw lastError ?? new Error("Transaction failed after nonce recovery retries");
}

export function toRpcError(error: unknown): RpcErrorShape {
  if (error && typeof error === "object") {
    const maybe = error as { code?: unknown; message?: unknown; data?: unknown; error?: unknown };
    if (maybe.error && typeof maybe.error === "object") {
      const nested = maybe.error as { code?: unknown; message?: unknown; data?: unknown };
      return {
        code: typeof nested.code === "number" ? nested.code : null,
        message: typeof nested.message === "string" ? nested.message : errorMessage(error),
        data: nested.data,
        raw: error
      };
    }

    return {
      code: typeof maybe.code === "number" ? maybe.code : null,
      message: typeof maybe.message === "string" ? maybe.message : errorMessage(error),
      data: maybe.data,
      raw: error
    };
  }

  return {
    code: null,
    message: errorMessage(error),
    raw: error
  };
}

async function sendTransactionWithFallback(
  wallet: Wallet,
  tx: { to?: string; data?: string; value?: bigint; gasLimit?: bigint }
): Promise<SendTransactionResult> {
  if (!wallet.provider) {
    throw new Error("Wallet provider is not attached");
  }

  const feeData = await wallet.provider.getFeeData();

  const maxPriorityFeePerGas = feeData.maxPriorityFeePerGas ?? feeData.gasPrice ?? 1_000_000_000n;
  const maxFeePerGas = feeData.maxFeePerGas ?? maxPriorityFeePerGas * 2n;

  const baseTx = {
    to: tx.to,
    data: tx.data,
    value: tx.value ?? 0n,
    gasLimit: tx.gasLimit
  };

  try {
    const { txResponse: eip1559Response, receipt: eip1559Receipt } = await sendWithNonceRecovery(
      wallet,
      {
        ...baseTx,
        type: 2,
        maxFeePerGas,
        maxPriorityFeePerGas
      }
    );

    return {
      txHash: eip1559Response.hash,
      receipt: {
        status: Number(eip1559Receipt.status ?? 0),
        logsLength: eip1559Receipt.logs.length,
        effectiveGasPricePresent: hasEffectiveGasPrice(eip1559Receipt),
        contractAddress: eip1559Receipt.contractAddress ?? null
      },
      txTypeUsed: "eip1559"
    };
  } catch (error) {
    if (!isLikely1559CompatibilityIssue(error)) {
      throw error;
    }

    const gasPrice = feeData.gasPrice ?? 1_000_000_000n;

    const { txResponse: legacyResponse, receipt: legacyReceipt } = await sendWithNonceRecovery(
      wallet,
      {
        ...baseTx,
        type: 0,
        gasPrice
      }
    );

    return {
      txHash: legacyResponse.hash,
      receipt: {
        status: Number(legacyReceipt.status ?? 0),
        logsLength: legacyReceipt.logs.length,
        effectiveGasPricePresent: hasEffectiveGasPrice(legacyReceipt),
        contractAddress: legacyReceipt.contractAddress ?? null
      },
      txTypeUsed: "legacy"
    };
  }
}

async function deployContract(
  wallet: Wallet,
  artifact: ContractArtifact,
  args: unknown[] = []
): Promise<{ address: string; deploymentReceiptContractAddressPresent: boolean }> {
  const factory = new ContractFactory(artifact.abi, artifact.bytecode, wallet);
  const deployRequest = await factory.getDeployTransaction(...args);
  const deployData = typeof deployRequest.data === "string" ? deployRequest.data : null;
  if (!deployData) {
    throw new Error("Deployment transaction data was missing");
  }

  const valueRaw = deployRequest.value;
  const deployResult = await sendTransactionWithFallback(wallet, {
    data: deployData,
    value: valueRaw ? BigInt(valueRaw.toString()) : 0n,
    gasLimit: DEPLOY_TX_GAS_LIMIT
  });
  const contractAddress = deployResult.receipt.contractAddress;
  if (!contractAddress) {
    throw new Error("Deployment receipt did not include contractAddress");
  }

  return {
    address: contractAddress,
    deploymentReceiptContractAddressPresent: true
  };
}

async function callContractWithFallback(
  wallet: Wallet,
  contract: Contract,
  method: string,
  args: unknown[]
): Promise<SendTransactionResult> {
  const data = contract.interface.encodeFunctionData(method, args);
  return sendTransactionWithFallback(wallet, {
    to: await contract.getAddress(),
    data,
    gasLimit: CONTRACT_TX_GAS_LIMIT
  });
}

async function seedState(runtime: EndpointRuntime, contracts: CompiledContracts): Promise<void> {
  const kitchen = new Contract(runtime.deployment.kitchenSink, contracts.kitchenSink.abi, runtime.wallet);

  await callContractWithFallback(runtime.wallet, kitchen, "setScalarBundle", [
    42_4242n,
    -77n,
    runtime.wallet.address,
    true
  ]);

  await callContractWithFallback(runtime.wallet, kitchen, "setSimpleMapping", [runtime.wallet.address, 1_111n]);

  await callContractWithFallback(runtime.wallet, kitchen, "setNestedMapping", [
    runtime.wallet.address,
    9n,
    "0x0f0e0d0c0b0a090807060504030201000102030405060708090a0b0c0d0e0f00"
  ]);

  await callContractWithFallback(runtime.wallet, kitchen, "setDynamicBundle", [
    "0x1234abcd",
    "kitchen-dynamic",
    [5n, 8n, 13n],
    {
      id: 99n,
      label: "embedded-struct",
      blob: "0xdeadbeef",
      enabled: false
    }
  ]);
}

async function deployAll(runtime: Omit<EndpointRuntime, "deployment">, contracts: CompiledContracts): Promise<DeploymentState> {
  const delegate = await deployContract(runtime.wallet, contracts.delegateTarget);
  const receiver = await deployContract(runtime.wallet, contracts.callReceiver);
  const kitchen = await deployContract(runtime.wallet, contracts.kitchenSink);

  return {
    kitchenSink: kitchen.address,
    delegateTarget: delegate.address,
    callReceiver: receiver.address,
    deploymentReceiptContractAddressPresent: kitchen.deploymentReceiptContractAddressPresent
  };
}

export async function prepareEndpoint(config: EndpointConfig, contracts: CompiledContracts): Promise<EndpointRuntime> {
  const provider = new JsonRpcProvider(config.rpcUrl);
  await provider.getNetwork();

  const wallet = new Wallet(config.privateKey, provider);
  const account = privateKeyToAccount(config.privateKey);
  const viemClient = createPublicClient({
    transport: http(config.rpcUrl, { timeout: 30_000, retryCount: 0 })
  });

  const baseRuntime = {
    name: config.name,
    rpcUrl: config.rpcUrl,
    chainId: config.chainId,
    provider,
    wallet,
    viemClient,
    account
  };

  const deployment = await deployAll(baseRuntime, contracts);
  const runtime: EndpointRuntime = {
    ...baseRuntime,
    deployment
  };

  await seedState(runtime, contracts);
  return runtime;
}

export async function sendContractTransactionWithFallback(
  runtime: EndpointRuntime,
  artifact: ContractArtifact,
  contractAddress: string,
  method: string,
  args: unknown[],
  value?: bigint
): Promise<SendTransactionResult> {
  const contract = new Contract(contractAddress, artifact.abi, runtime.wallet);
  const data = contract.interface.encodeFunctionData(method, args);
  return sendTransactionWithFallback(runtime.wallet, {
    to: contractAddress,
    data,
    value,
    gasLimit: CONTRACT_TX_GAS_LIMIT
  });
}

export async function sendValueTransactionWithFallback(
  runtime: EndpointRuntime,
  to: string,
  value: bigint
): Promise<SendTransactionResult> {
  return sendTransactionWithFallback(runtime.wallet, {
    to,
    value,
    gasLimit: VALUE_TX_GAS_LIMIT
  });
}

export function createContract(runtime: EndpointRuntime, artifact: ContractArtifact, address: string): Contract {
  return new Contract(address, artifact.abi, runtime.wallet);
}
