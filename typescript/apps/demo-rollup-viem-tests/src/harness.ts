import { readFile } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import {
  createPublicClient,
  createWalletClient,
  defineChain,
  http,
  type Abi,
  type Address,
  type Chain,
  type Hex,
  type HttpTransport,
  type PublicClient,
  type WalletClient
} from "viem";
import { privateKeyToAccount, type PrivateKeyAccount } from "viem/accounts";

export const DEFAULT_RPC_URL = process.env.DEMO_ROLLUP_RPC_URL ?? "http://127.0.0.1:12346/rpc";
export const DEFAULT_DEPLOYER_PRIVATE_KEY =
  process.env.DEMO_ROLLUP_DEPLOYER_PRIVATE_KEY ??
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
export const DEFAULT_UNFUNDED_PRIVATE_KEY =
  process.env.DEMO_ROLLUP_UNFUNDED_PRIVATE_KEY ??
  "0x1000000000000000000000000000000000000000000000000000000000000001";

export const DEMO_ROLLUP_CHAIN_ID = 4321;
export const MAX_FEE_PER_GAS = 100n;
export const MAX_PRIORITY_FEE_PER_GAS = 1n;
export const RECEIPT_TIMEOUT_MS = 60_000;

const RPC_READY_TIMEOUT_MS = 20_000;

export interface DemoRollupRuntime {
  rpcUrl: string;
  chain: Chain;
  publicClient: PublicClient<HttpTransport, Chain>;
  deployer: PrivateKeyAccount;
  deployerWallet: WalletClient<HttpTransport, Chain, PrivateKeyAccount>;
  unfunded: PrivateKeyAccount;
  unfundedWallet: WalletClient<HttpTransport, Chain, PrivateKeyAccount>;
  abi: Abi;
  bytecode: Hex;
}

function packageRoot(): string {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
}

function repoRoot(): string {
  return path.resolve(packageRoot(), "../../..");
}

function artifactPath(filename: string): string {
  return path.join(
    repoRoot(),
    "crates",
    "utils",
    "sov-evm-test-utils",
    "contracts",
    "artifacts",
    filename
  );
}

function normalizePrivateKey(raw: string): `0x${string}` {
  const normalized = raw.startsWith("0x") ? raw : `0x${raw}`;
  if (!/^0x[0-9a-fA-F]{64}$/.test(normalized)) {
    throw new Error(`Invalid private key: expected 32-byte hex, got ${raw}`);
  }
  return normalized as `0x${string}`;
}

function normalizeHex(raw: string): Hex {
  const trimmed = raw.trim();
  const normalized = trimmed.startsWith("0x") ? trimmed : `0x${trimmed}`;
  if (!/^0x[0-9a-fA-F]*$/.test(normalized)) {
    throw new Error(`Invalid hex payload in artifact: ${normalized.slice(0, 32)}...`);
  }
  return normalized as Hex;
}

function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return JSON.stringify(error);
}

async function loadSimpleStorageAbi(): Promise<Abi> {
  const contents = await readFile(artifactPath("SimpleStorage.abi"), "utf8");
  return JSON.parse(contents) as Abi;
}

async function loadSimpleStorageBytecode(): Promise<Hex> {
  const contents = await readFile(artifactPath("SimpleStorage.bin"), "utf8");
  return normalizeHex(contents);
}

async function rpcChainId(url: string): Promise<number | null> {
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json"
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "eth_chainId",
      params: []
    })
  });

  if (!response.ok) {
    return null;
  }

  const body = (await response.json()) as { result?: unknown };
  if (typeof body.result !== "string" || !/^0x[0-9a-fA-F]+$/.test(body.result)) {
    return null;
  }

  return Number(BigInt(body.result));
}

export async function waitForRpcReady(
  url: string,
  timeoutMs = RPC_READY_TIMEOUT_MS
): Promise<void> {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    try {
      const chainId = await rpcChainId(url);
      if (chainId !== null) {
        return;
      }
    } catch {
      // Retry until timeout.
    }

    await delay(300);
  }

  throw new Error(`RPC endpoint did not become ready within ${timeoutMs}ms: ${url}`);
}

export async function createDemoRollupRuntime(): Promise<DemoRollupRuntime> {
  const rpcUrl = DEFAULT_RPC_URL;
  const deployer = privateKeyToAccount(normalizePrivateKey(DEFAULT_DEPLOYER_PRIVATE_KEY));
  const unfunded = privateKeyToAccount(normalizePrivateKey(DEFAULT_UNFUNDED_PRIVATE_KEY));

  await waitForRpcReady(rpcUrl);

  const bootstrapClient = createPublicClient({
    transport: http(rpcUrl)
  });
  const chainId = await bootstrapClient.getChainId();

  if (chainId !== DEMO_ROLLUP_CHAIN_ID) {
    throw new Error(
      `Expected demo-rollup chain ID ${DEMO_ROLLUP_CHAIN_ID}, got ${chainId}. ` +
        `This suite targets the default standalone demo-rollup config at ${DEFAULT_RPC_URL}.`
    );
  }

  const chain = defineChain({
    id: chainId,
    name: "demo-rollup",
    nativeCurrency: {
      name: "Ether",
      symbol: "ETH",
      decimals: 18
    },
    rpcUrls: {
      default: { http: [rpcUrl] },
      public: { http: [rpcUrl] }
    }
  });

  const publicClient = createPublicClient({
    chain,
    transport: http(rpcUrl)
  });
  const deployerWallet = createWalletClient({
    account: deployer,
    chain,
    transport: http(rpcUrl)
  });
  const unfundedWallet = createWalletClient({
    account: unfunded,
    chain,
    transport: http(rpcUrl)
  });

  const [abi, bytecode, deployerBalance, unfundedBalance] = await Promise.all([
    loadSimpleStorageAbi(),
    loadSimpleStorageBytecode(),
    publicClient.getBalance({ address: deployer.address }),
    publicClient.getBalance({ address: unfunded.address })
  ]);

  if (deployerBalance === 0n) {
    throw new Error(
      `Expected deployer ${deployer.address} to be funded on ${rpcUrl}. ` +
        "This suite targets the default standalone demo-rollup demo/mock genesis."
    );
  }

  if (unfundedBalance !== 0n) {
    throw new Error(
      `Expected unfunded sender ${unfunded.address} to start with zero balance, got ${unfundedBalance}. ` +
        "Override DEMO_ROLLUP_UNFUNDED_PRIVATE_KEY if this key is funded on your target."
    );
  }

  return {
    rpcUrl,
    chain,
    publicClient,
    deployer,
    deployerWallet,
    unfunded,
    unfundedWallet,
    abi,
    bytecode
  };
}

export async function deploySimpleStorage(
  runtime: DemoRollupRuntime
): Promise<Address> {
  const nonce = await runtime.publicClient.getTransactionCount({
    address: runtime.deployer.address,
    blockTag: "pending"
  });

  const gas = await runtime.publicClient.estimateGas({
    account: runtime.deployer.address,
    data: runtime.bytecode,
    maxFeePerGas: MAX_FEE_PER_GAS,
    maxPriorityFeePerGas: MAX_PRIORITY_FEE_PER_GAS,
    nonce
  });

  const hash = await runtime.deployerWallet.sendTransaction({
    account: runtime.deployer,
    data: runtime.bytecode,
    gas,
    maxFeePerGas: MAX_FEE_PER_GAS,
    maxPriorityFeePerGas: MAX_PRIORITY_FEE_PER_GAS,
    nonce
  });

  const receipt = await runtime.publicClient.waitForTransactionReceipt({
    hash,
    timeout: RECEIPT_TIMEOUT_MS
  });

  if (receipt.status !== "success" || !receipt.contractAddress) {
    throw new Error(
      `SimpleStorage deployment failed on ${runtime.rpcUrl}: ${describeError(receipt)}`
    );
  }

  return receipt.contractAddress;
}

export function getEstimateMismatchAllowance(estimate: bigint): bigint {
  const percentageAllowance = estimate / 5n;
  return percentageAllowance > 5_000n ? percentageAllowance : 5_000n;
}

export function formatEstimateFailure(subcase: string, sender: Address, error: unknown): string {
  return (
    `${subcase} estimateGas failed for unfunded sender ${sender}. ` +
    "This suite expects the paymaster-enabled demo/mock genesis. " +
    describeError(error)
  );
}
