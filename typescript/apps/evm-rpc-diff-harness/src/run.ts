import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import dotenv from "dotenv";
import { loadCompiledContracts, prepareEndpoint } from "./harness/deploy";
import { runChecks } from "./harness/checks";
import { writeReport } from "./harness/report";
import { JsonRpcClient } from "./harness/rpc";
import type { EndpointConfig } from "./harness/types";

dotenv.config();

const DEFAULT_TIMEOUT_ERROR =
  "Missing required RPC configuration. Pass CLI args or set ANVIL_RPC_URL, ROLLUP_RPC_URL, and TEST_PRIVATE_KEY.";

export interface ComparisonOptions {
  anvilRpcUrl: string;
  rollupRpcUrl: string;
  privateKey: `0x${string}`;
  chainIdAnvil?: bigint;
  chainIdRollup?: bigint;
}

function parseChainId(value: string): bigint {
  if (value.startsWith("0x") || value.startsWith("0X")) {
    return BigInt(value);
  }
  if (!/^\d+$/.test(value)) {
    throw new Error(`Invalid chain id format: ${value}`);
  }
  return BigInt(value);
}

function parsePrivateKey(value: string): `0x${string}` {
  const normalized = value.startsWith("0x") ? value : `0x${value}`;
  if (!/^0x[0-9a-fA-F]{64}$/.test(normalized)) {
    throw new Error("TEST_PRIVATE_KEY must be a 32-byte hex private key");
  }
  return normalized as `0x${string}`;
}

async function detectChainId(rpcUrl: string): Promise<bigint> {
  const client = new JsonRpcClient(rpcUrl);
  const response = await client.call("eth_chainId", []);
  if (!response.ok) {
    throw new Error(`Failed to detect chain id for ${rpcUrl}: ${response.error.message}`);
  }

  if (typeof response.result !== "string" || !/^0x[0-9a-fA-F]+$/.test(response.result)) {
    throw new Error(`Invalid eth_chainId result for ${rpcUrl}: ${JSON.stringify(response.result)}`);
  }

  return BigInt(response.result);
}

async function detectClientVersion(rpcUrl: string): Promise<string | undefined> {
  const client = new JsonRpcClient(rpcUrl);
  const response = await client.call("web3_clientVersion", []);
  if (!response.ok) {
    return undefined;
  }

  return typeof response.result === "string" ? response.result : undefined;
}

function resolveOptionsFromArgs(argv: string[]): ComparisonOptions {
  // `pnpm run <script> -- <args>` can inject one or more standalone `--` tokens.
  // Strip them so Node's parser doesn't treat following options as positionals.
  const sanitizedArgv = argv.filter((arg) => arg !== "--");

  const parsed = parseArgs({
    args: sanitizedArgv,
    options: {
      anvil: { type: "string" },
      rollup: { type: "string" },
      pk: { type: "string" },
      "chain-id-anvil": { type: "string" },
      "chain-id-rollup": { type: "string" }
    },
    allowPositionals: false
  });

  const anvilRpcUrl = parsed.values.anvil ?? process.env.ANVIL_RPC_URL;
  const rollupRpcUrl = parsed.values.rollup ?? process.env.ROLLUP_RPC_URL;
  const privateKeyRaw = parsed.values.pk ?? process.env.TEST_PRIVATE_KEY;

  if (!anvilRpcUrl || !rollupRpcUrl || !privateKeyRaw) {
    throw new Error(DEFAULT_TIMEOUT_ERROR);
  }

  const chainIdAnvilRaw = parsed.values["chain-id-anvil"] ?? process.env.CHAIN_ID_ANVIL;
  const chainIdRollupRaw = parsed.values["chain-id-rollup"] ?? process.env.CHAIN_ID_ROLLUP;

  return {
    anvilRpcUrl,
    rollupRpcUrl,
    privateKey: parsePrivateKey(privateKeyRaw),
    chainIdAnvil: chainIdAnvilRaw ? parseChainId(chainIdAnvilRaw) : undefined,
    chainIdRollup: chainIdRollupRaw ? parseChainId(chainIdRollupRaw) : undefined
  };
}

function appRootFromRunFile(): string {
  const runFile = fileURLToPath(import.meta.url);
  return path.resolve(path.dirname(runFile), "..");
}

export async function runComparison(options: ComparisonOptions): Promise<void> {
  const appRoot = appRootFromRunFile();

  const chainIdAnvil = options.chainIdAnvil ?? (await detectChainId(options.anvilRpcUrl));
  const chainIdRollup = options.chainIdRollup ?? (await detectChainId(options.rollupRpcUrl));

  const contracts = await loadCompiledContracts(appRoot);

  const anvilConfig: EndpointConfig = {
    name: "anvil",
    rpcUrl: options.anvilRpcUrl,
    privateKey: options.privateKey,
    chainId: chainIdAnvil
  };

  const rollupConfig: EndpointConfig = {
    name: "rollup",
    rpcUrl: options.rollupRpcUrl,
    privateKey: options.privateKey,
    chainId: chainIdRollup
  };

  // Deploy sequentially to avoid nonce races when both targets share one RPC endpoint.
  const anvilRuntime = await prepareEndpoint(anvilConfig, contracts);
  const rollupRuntime = await prepareEndpoint(rollupConfig, contracts);
  const [anvilClientVersion, rollupClientVersion] = await Promise.all([
    detectClientVersion(options.anvilRpcUrl),
    detectClientVersion(options.rollupRpcUrl)
  ]);

  const checks = await runChecks({
    contracts,
    anvil: anvilRuntime,
    rollup: rollupRuntime
  });

  const report = await writeReport(
    appRoot,
    checks,
    {
      rpcUrl: options.anvilRpcUrl,
      chainId: chainIdAnvil,
      clientVersion: anvilClientVersion
    },
    {
      rpcUrl: options.rollupRpcUrl,
      chainId: chainIdRollup,
      clientVersion: rollupClientVersion
    }
  );

  const reportJson = path.join(appRoot, "artifacts", "report.json");
  const reportMd = path.join(appRoot, "artifacts", "report.md");

  console.log("Comparison complete.");
  console.log(`PASS=${report.summary.pass} FAIL=${report.summary.fail} NOT_SUPPORTED=${report.summary.notSupported}`);
  console.log(`JSON report: ${reportJson}`);
  console.log(`Markdown report: ${reportMd}`);

  if (report.summary.fail > 0) {
    throw new Error(
      `Comparison reported ${report.summary.fail} failing check(s). See reports at ${reportJson} and ${reportMd}.`
    );
  }
}

async function main(): Promise<void> {
  try {
    const options = resolveOptionsFromArgs(process.argv.slice(2));
    await runComparison(options);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`compare failed: ${message}`);
    process.exitCode = 1;
  }
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : null;
const currentPath = fileURLToPath(import.meta.url);
if (invokedPath !== null && currentPath === invokedPath) {
  void main();
}
