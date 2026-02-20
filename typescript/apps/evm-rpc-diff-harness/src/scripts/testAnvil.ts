import { spawn } from "node:child_process";
import { once } from "node:events";
import { Wallet } from "ethers";
import { waitForRpcReady } from "../harness/rpc";
import { runComparison } from "../run";

const DEFAULT_ANVIL_PK =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const DEFAULT_ANVIL_URL = "http://127.0.0.1:8545";
const DEFAULT_ANVIL_PORT = "8545";
const DEFAULT_ANVIL_LOG_TAIL_LINES = 120;

function normalizePrivateKey(raw: string): `0x${string}` {
  const normalized = raw.startsWith("0x") ? raw : `0x${raw}`;
  if (!/^0x[0-9a-fA-F]{64}$/.test(normalized)) {
    throw new Error("TEST_PRIVATE_KEY must be 32-byte hex");
  }
  return normalized as `0x${string}`;
}

function resolveAnvilBinding(rpcUrl: string): { rpcUrl: string; host: string; port: string } {
  let parsed: URL;
  try {
    parsed = new URL(rpcUrl);
  } catch {
    throw new Error(`ANVIL_RPC_URL must be a valid URL: ${rpcUrl}`);
  }

  if (parsed.protocol !== "http:") {
    throw new Error(`ANVIL_RPC_URL must use http:// for local test:anvil mode: ${rpcUrl}`);
  }

  if (!parsed.hostname) {
    throw new Error(`ANVIL_RPC_URL must include a hostname: ${rpcUrl}`);
  }

  if (parsed.pathname !== "/" || parsed.search || parsed.hash) {
    throw new Error(
      `ANVIL_RPC_URL must not include path/query/hash in local test:anvil mode: ${rpcUrl}`
    );
  }

  if (!parsed.port) {
    parsed.port = DEFAULT_ANVIL_PORT;
  }

  return {
    rpcUrl: parsed.toString(),
    host: parsed.hostname,
    port: parsed.port
  };
}

type AnvilLogSource = "stdout" | "stderr";

function parseBooleanFlag(value: string | undefined): boolean {
  if (!value) {
    return false;
  }
  const normalized = value.trim().toLowerCase();
  return normalized === "1" || normalized === "true" || normalized === "yes";
}

function createAnvilLogCollector(verbose: boolean, maxTailLines: number): {
  consume: (source: AnvilLogSource, chunk: Buffer) => void;
  flush: () => void;
  totalLines: () => number;
  tail: () => string[];
} {
  const tailLines: string[] = [];
  let total = 0;
  let stdoutRemainder = "";
  let stderrRemainder = "";

  function pushLine(source: AnvilLogSource, line: string): void {
    const entry = `[${source}] ${line}`;
    total += 1;
    tailLines.push(entry);
    if (tailLines.length > maxTailLines) {
      tailLines.shift();
    }

    if (verbose) {
      const out = source === "stdout" ? process.stdout : process.stderr;
      out.write(`[anvil] ${line}\n`);
    }
  }

  function consume(source: AnvilLogSource, chunk: Buffer): void {
    const previous = source === "stdout" ? stdoutRemainder : stderrRemainder;
    const text = `${previous}${chunk.toString()}`;
    const lines = text.split(/\r?\n/);
    const remainder = lines.pop() ?? "";

    for (const line of lines) {
      pushLine(source, line);
    }

    if (source === "stdout") {
      stdoutRemainder = remainder;
    } else {
      stderrRemainder = remainder;
    }
  }

  function flush(): void {
    if (stdoutRemainder.length > 0) {
      pushLine("stdout", stdoutRemainder);
      stdoutRemainder = "";
    }
    if (stderrRemainder.length > 0) {
      pushLine("stderr", stderrRemainder);
      stderrRemainder = "";
    }
  }

  return {
    consume,
    flush,
    totalLines: () => total,
    tail: () => [...tailLines]
  };
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
  const configuredAnvilUrl = process.env.ANVIL_RPC_URL ?? DEFAULT_ANVIL_URL;
  const { rpcUrl: anvilUrl, host: anvilHost, port: anvilPort } = resolveAnvilBinding(configuredAnvilUrl);
  const rollupUrl = process.env.ROLLUP_RPC_URL ?? anvilUrl;
  const privateKey = normalizePrivateKey(process.env.TEST_PRIVATE_KEY ?? DEFAULT_ANVIL_PK);
  const verboseAnvilLogs = parseBooleanFlag(process.env.ANVIL_VERBOSE_LOGS);
  const logCollector = createAnvilLogCollector(verboseAnvilLogs, DEFAULT_ANVIL_LOG_TAIL_LINES);

  const anvil = spawn(
    "anvil",
    ["--host", anvilHost, "--port", anvilPort, "--chain-id", process.env.CHAIN_ID_ANVIL ?? "31337"],
    {
      stdio: ["ignore", "pipe", "pipe"]
    }
  );

  anvil.stdout.on("data", (chunk) => {
    logCollector.consume("stdout", chunk);
  });
  anvil.stderr.on("data", (chunk) => {
    logCollector.consume("stderr", chunk);
  });

  anvil.on("error", (error) => {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      console.error("anvil binary is not installed or not in PATH.");
    } else {
      console.error(`failed to launch anvil: ${error.message}`);
    }
  });

  let completed = false;
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
    completed = true;
  } catch (error) {
    logCollector.flush();
    if (!verboseAnvilLogs) {
      const tail = logCollector.tail();
      if (tail.length > 0) {
        console.error(`Anvil logs (last ${tail.length} lines):`);
        for (const line of tail) {
          console.error(`[anvil] ${line}`);
        }
      }
    }
    throw error;
  } finally {
    logCollector.flush();
    await stopProcess(anvil);
    if (completed && !verboseAnvilLogs) {
      const totalLines = logCollector.totalLines();
      if (totalLines > 0) {
        console.log(
          `Suppressed ${totalLines} anvil log line(s). Set ANVIL_VERBOSE_LOGS=1 to stream raw anvil logs.`
        );
      }
    }
  }
}

main().catch((error) => {
  console.error(`test:anvil failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
