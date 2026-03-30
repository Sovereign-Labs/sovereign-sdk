import {
  generateKeyPairSync,
  sign,
  verify,
  type KeyObject,
} from "node:crypto";
import { Buffer } from "node:buffer";
import { expect, test } from "@playwright/test";

const ROLLUP_URL = process.env.VITE_ROLLUP_URL ?? "http://localhost:12346";
const SOLANA_ENDPOINT =
  process.env.VITE_SOLANA_ENDPOINT || "/sequencer/accept-solana-offchain-tx";
const CHAIN_ID = Number(process.env.VITE_CHAIN_ID ?? "4321");
const BASE58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

type SolanaSimpleEnvelope = {
  signedMessage: Buffer;
  chainHashHex: string;
  pubkeyHex: string;
  signature: Buffer;
};

type SolanaUnsignedTx = {
  chain_name?: string;
  details?: {
    chain_id?: number;
  };
  runtime_call?: {
    bank?: {
      create_token?: {
        mint_to_address?: string;
        initial_balance?: string;
        supply_cap?: string;
      };
    };
  };
};

function base58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) {
    return "";
  }

  let value = BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
  let encoded = "";

  while (value > 0n) {
    const mod = Number(value % 58n);
    encoded = BASE58_ALPHABET[mod] + encoded;
    value /= 58n;
  }

  let leadingZeros = 0;
  while (leadingZeros < bytes.length && bytes[leadingZeros] === 0) {
    leadingZeros += 1;
  }

  return "1".repeat(leadingZeros) + encoded;
}

function publicKeyToRawBytes(publicKey: KeyObject): Buffer {
  const der = publicKey.export({ type: "spki", format: "der" });
  if (!Buffer.isBuffer(der) || der.length < 32) {
    throw new Error("Failed to export Ed25519 public key bytes");
  }
  return der.subarray(der.length - 32);
}

function parseSolanaSimpleEnvelope(payloadBase64: string): SolanaSimpleEnvelope {
  const serialized = Buffer.from(payloadBase64, "base64");
  const minEnvelopeSize = 4 + 32 + 32 + 64;
  if (serialized.length < minEnvelopeSize) {
    throw new Error(
      `Serialized message too short: ${serialized.length} bytes (minimum ${minEnvelopeSize})`,
    );
  }

  const signedMessageLength = serialized.readUInt32LE(0);
  const signedMessageStart = 4;
  const signedMessageEnd = signedMessageStart + signedMessageLength;
  const chainHashStart = signedMessageEnd;
  const chainHashEnd = chainHashStart + 32;
  const pubkeyStart = chainHashEnd;
  const pubkeyEnd = pubkeyStart + 32;
  const signatureStart = pubkeyEnd;
  const signatureEnd = signatureStart + 64;

  if (signatureEnd !== serialized.length) {
    throw new Error(
      `Invalid Solana envelope length: expected ${signatureEnd}, got ${serialized.length}`,
    );
  }

  return {
    signedMessage: serialized.subarray(signedMessageStart, signedMessageEnd),
    chainHashHex: serialized.subarray(chainHashStart, chainHashEnd).toString("hex"),
    pubkeyHex: serialized.subarray(pubkeyStart, pubkeyEnd).toString("hex"),
    signature: serialized.subarray(signatureStart, signatureEnd),
  };
}

async function assertRollupIsReachable() {
  const healthUrl = new URL("/healthcheck", ROLLUP_URL).toString();

  let response: Response;
  try {
    response = await fetch(healthUrl);
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    throw new Error(
      `Rollup node is not reachable at ${ROLLUP_URL}. Start the demo rollup before running e2e.\n${message}`,
    );
  }

  if (!response.ok) {
    throw new Error(
      `Rollup healthcheck failed at ${healthUrl} with status ${response.status}`,
    );
  }
}

async function fetchRollupChainHash(): Promise<string> {
  const schemaUrl = new URL("/rollup/schema", ROLLUP_URL).toString();
  const response = await fetch(schemaUrl);

  if (!response.ok) {
    throw new Error(
      `Failed to fetch rollup schema at ${schemaUrl}: ${response.status}`,
    );
  }

  const body = (await response.json()) as { chain_hash?: string };
  if (typeof body.chain_hash !== "string") {
    throw new Error(`Unexpected schema response from ${schemaUrl}`);
  }

  return body.chain_hash.replace(/^0x/, "");
}

test.describe("Phantom mocked-provider with real rollup", () => {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const publicKeyRaw = publicKeyToRawBytes(publicKey);
  const walletAddress = base58Encode(new Uint8Array(publicKeyRaw));

  test.beforeEach(async ({ page }) => {
    await page.exposeFunction("__signEd25519Message", async (message: number[]) => {
      const signature = sign(null, Buffer.from(message), privateKey);
      return Array.from(signature);
    });

    await page.addInitScript(
      ({ address, publicKeyBytes }) => {
        const publicKey = {
          toString: () => address,
          toBytes: () => new Uint8Array(publicKeyBytes),
        };

        let isTrusted = false;
        let isConnected = false;

        const provider = {
          isPhantom: true,
          isConnected: false,
          publicKey: null as typeof publicKey | null,
          connect: async (options?: { onlyIfTrusted?: boolean }) => {
            if (options?.onlyIfTrusted && !isTrusted) {
              throw new Error("Provider is not trusted");
            }

            isTrusted = true;
            isConnected = true;
            provider.isConnected = true;
            provider.publicKey = publicKey;
            return { publicKey };
          },
          disconnect: async () => {
            isConnected = false;
            provider.isConnected = false;
            provider.publicKey = null;
          },
          signMessage: async (message: Uint8Array) => {
            if (!isConnected) {
              throw new Error("Phantom wallet is not connected");
            }

            const signatureBytes = await (
              window as Window & {
                __signEd25519Message: (bytes: number[]) => Promise<number[]>;
              }
            ).__signEd25519Message(Array.from(message));

            return { signature: new Uint8Array(signatureBytes) };
          },
        };

        Object.defineProperty(window, "phantom", {
          value: { solana: provider },
          configurable: true,
        });

        Object.defineProperty(window, "solana", {
          value: provider,
          configurable: true,
        });
      },
      { address: walletAddress, publicKeyBytes: Array.from(publicKeyRaw) },
    );
  });

  test("submits a Solana offchain tx through real rollup endpoint", async ({
    page,
  }) => {
    await assertRollupIsReachable();

    const submitEndpoint = new URL(SOLANA_ENDPOINT, ROLLUP_URL).toString();
    const submissionRequestPromise = page.waitForRequest(
      (req) => req.method() === "POST" && req.url() === submitEndpoint,
    );

    await page.goto("/");

    await expect(
      page.getByRole("heading", { name: "Phantom Solana Offchain Example" }),
    ).toBeVisible();

    await expect(
      page.getByText("Phantom was not detected. Install the Phantom browser extension"),
    ).toHaveCount(0);

    await page.getByRole("button", { name: "Connect Phantom" }).click();
    await expect(page.getByRole("button", { name: /Disconnect/ })).toBeVisible({
      timeout: 15_000,
    });

    await page.getByRole("button", { name: "Sign with Phantom and Send" }).click();

    const submissionRequest = await submissionRequestPromise;
    const postBody = submissionRequest.postData();
    if (!postBody) {
      throw new Error("Expected request body for solana offchain tx submission");
    }

    const parsedBody = JSON.parse(postBody) as { body?: string };
    if (typeof parsedBody.body !== "string") {
      throw new Error(
        `Expected AcceptTx payload shape {"body":"<base64>"}, got: ${postBody.slice(0, 300)}`,
      );
    }

    const envelope = parseSolanaSimpleEnvelope(parsedBody.body);
    const unsignedTx = JSON.parse(envelope.signedMessage.toString("utf8")) as SolanaUnsignedTx;

    const liveChainHash = await fetchRollupChainHash();
    expect(envelope.chainHashHex).toBe(liveChainHash);
    expect(envelope.pubkeyHex).toBe(publicKeyRaw.toString("hex"));
    expect(
      verify(null, envelope.signedMessage, publicKey, envelope.signature),
    ).toBeTruthy();

    expect(unsignedTx.chain_name).toBe("TestChain");
    expect(unsignedTx.details?.chain_id).toBe(CHAIN_ID);
    expect(unsignedTx.runtime_call?.bank?.create_token?.mint_to_address).toBe(
      walletAddress,
    );
    expect(unsignedTx.runtime_call?.bank?.create_token?.initial_balance).toBe(
      "1000000000",
    );
    expect(unsignedTx.runtime_call?.bank?.create_token?.supply_cap).toBe(
      "100000000000",
    );

    const successMessage = page.locator(".message.success");
    const errorMessage = page.locator(".message.error");
    await expect(successMessage.or(errorMessage)).toBeVisible({
      timeout: 60_000,
    });

    if (await errorMessage.isVisible()) {
      const errorText = (await errorMessage.innerText()) || "Unknown error";
      throw new Error(`Rollup rejected transaction:\n${errorText}`);
    }

    await expect(successMessage).toContainText("Transaction Submitted Successfully!");
  });
});
