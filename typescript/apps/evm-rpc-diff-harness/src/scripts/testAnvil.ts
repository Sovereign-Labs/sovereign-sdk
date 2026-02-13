import { spawn } from "node:child_process";
import { once } from "node:events";
import { Wallet } from "ethers";
import { waitForRpcReady } from "../harness/rpc";
import { runComparison } from "../run";

const DEFAULT_ANVIL_PK =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const DEFAULT_ANVIL_URL = "http://127.0.0.1:8545";

function normalizePrivateKey(raw: string): `0x${string}` {
  const normalized = raw.startsWith("0x") ? raw : `0x${raw}`;
  if (!/^0x[0-9a-fA-F]{64}$/.test(normalized)) {
    throw new Error("TEST_PRIVATE_KEY must be 32-byte hex");
  }
  return normalized as `0x${string}`;
}

async function stopProcess(child: ReturnType<typeof spawn>): Promise<void> {
  if (child.exitCode !== null) {
    return;
  }

  child.kill("SIGTERM");
  const exitPromise = once(child, "exit");
  const timeout = new Promise<void>((resolve) => {
    setTimeout(() => {
      if (child.exitCode === null) {
        child.kill("SIGKILL");
      }
      resolve();
    }, 3_000);
  });

  await Promise.race([exitPromise, timeout]);
}

async function main(): Promise<void> {
  const anvilUrl = process.env.ANVIL_RPC_URL ?? DEFAULT_ANVIL_URL;
  const rollupUrl = process.env.ROLLUP_RPC_URL ?? anvilUrl;
  const privateKey = normalizePrivateKey(process.env.TEST_PRIVATE_KEY ?? DEFAULT_ANVIL_PK);

  const anvil = spawn(
    "anvil",
    ["--host", "127.0.0.1", "--port", "8545", "--chain-id", process.env.CHAIN_ID_ANVIL ?? "31337"],
    {
      stdio: ["ignore", "pipe", "pipe"]
    }
  );

  anvil.stdout.on("data", (chunk) => {
    process.stdout.write(`[anvil] ${chunk.toString()}`);
  });
  anvil.stderr.on("data", (chunk) => {
    process.stderr.write(`[anvil] ${chunk.toString()}`);
  });

  anvil.on("error", (error) => {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      console.error("anvil binary is not installed or not in PATH.");
    } else {
      console.error(`failed to launch anvil: ${error.message}`);
    }
  });

  try {
    await waitForRpcReady(anvilUrl, 20_000);

    const fundedWallet = new Wallet(privateKey);
    console.log("Started local anvil baseline.");
    console.log(`Funded test account address: ${fundedWallet.address}`);
    console.log(`Funded test account private key: ${privateKey}`);
    console.log(`Anvil RPC URL: ${anvilUrl}`);
    console.log(`Rollup RPC URL: ${rollupUrl}`);

    await runComparison({
      anvilRpcUrl: anvilUrl,
      rollupRpcUrl: rollupUrl,
      privateKey,
      chainIdAnvil: process.env.CHAIN_ID_ANVIL ? BigInt(process.env.CHAIN_ID_ANVIL) : undefined,
      chainIdRollup: process.env.CHAIN_ID_ROLLUP ? BigInt(process.env.CHAIN_ID_ROLLUP) : undefined
    });
  } finally {
    await stopProcess(anvil);
  }
}

main().catch((error) => {
  console.error(`test:anvil failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
