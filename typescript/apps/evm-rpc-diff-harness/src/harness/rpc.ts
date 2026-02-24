import { setTimeout as delay } from "node:timers/promises";
import type { RpcErrorShape } from "./types";

let requestId = 1;

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params: unknown[];
}

export interface RpcCallSuccess {
  ok: true;
  request: JsonRpcRequest;
  response: unknown;
  result: unknown;
  durationMs: number;
}

export interface RpcCallFailure {
  ok: false;
  request: JsonRpcRequest;
  response: unknown;
  error: RpcErrorShape;
  durationMs: number;
}

export type RpcCallResult = RpcCallSuccess | RpcCallFailure;

export interface BatchCallInput {
  method: string;
  params?: unknown[];
}

export interface RpcBatchResult {
  supported: boolean;
  request: JsonRpcRequest[];
  responses: Array<{
    id: number;
    method?: string;
    ok: boolean;
    result?: unknown;
    error?: RpcErrorShape;
    raw: unknown;
  }>;
  raw: unknown;
}

function toRpcError(error: unknown, fallbackMessage: string): RpcErrorShape {
  if (error && typeof error === "object") {
    const maybe = error as { code?: unknown; message?: unknown; data?: unknown };
    const code = typeof maybe.code === "number" ? maybe.code : null;
    const message = typeof maybe.message === "string" ? maybe.message : fallbackMessage;
    return { code, message, data: maybe.data, raw: error };
  }

  return { code: null, message: fallbackMessage, raw: error };
}

export function isHexQuantity(value: unknown): value is string {
  return typeof value === "string" && /^0x[0-9a-fA-F]+$/.test(value);
}

export function decodeHexQuantity(value: unknown): bigint | null {
  if (!isHexQuantity(value)) {
    return null;
  }
  return BigInt(value);
}

export function isNotSupportedError(error: RpcErrorShape | undefined): boolean {
  if (!error) {
    return false;
  }

  const msg = error.message.toLowerCase();
  const unsupportedCode = error.code === -32601 || error.code === -32004;
  const unsupportedMessage =
    msg.includes("method not found") ||
    msg.includes("not supported") ||
    msg.includes("unsupported") ||
    msg.includes("does not exist");

  return unsupportedCode || unsupportedMessage;
}

export class JsonRpcClient {
  readonly url: string;

  constructor(url: string) {
    this.url = url;
  }

  async call(method: string, params: unknown[] = []): Promise<RpcCallResult> {
    const payload: JsonRpcRequest = {
      jsonrpc: "2.0",
      id: requestId++,
      method,
      params
    };

    const startedAt = Date.now();
    const response = await fetch(this.url, {
      method: "POST",
      headers: {
        "content-type": "application/json"
      },
      body: JSON.stringify(payload)
    });

    const durationMs = Date.now() - startedAt;
    const responseText = await response.text();

    let parsed: unknown;
    try {
      parsed = JSON.parse(responseText);
    } catch (error) {
      return {
        ok: false,
        request: payload,
        response: responseText,
        error: {
          code: response.status,
          message: `Non-JSON response (${response.status})`,
          data: responseText,
          raw: error
        },
        durationMs
      };
    }

    if (!response.ok) {
      return {
        ok: false,
        request: payload,
        response: parsed,
        error: {
          code: response.status,
          message: `HTTP ${response.status}`,
          data: parsed,
          raw: parsed
        },
        durationMs
      };
    }

    if (parsed && typeof parsed === "object") {
      const envelope = parsed as { result?: unknown; error?: unknown };
      const hasResult = "result" in envelope;
      const hasError = "error" in envelope && envelope.error !== null && envelope.error !== undefined;

      if (hasError) {
        return {
          ok: false,
          request: payload,
          response: parsed,
          error: toRpcError(envelope.error, "RPC call failed"),
          durationMs
        };
      }

      if (hasResult) {
        return {
          ok: true,
          request: payload,
          response: parsed,
          result: envelope.result,
          durationMs
        };
      }
    }

    return {
      ok: false,
      request: payload,
      response: parsed,
      error: {
        code: null,
        message: "Malformed JSON-RPC response",
        data: parsed,
        raw: parsed
      },
      durationMs
    };
  }

  async batch(calls: BatchCallInput[]): Promise<RpcBatchResult> {
    const payload = calls.map((call) => ({
      jsonrpc: "2.0" as const,
      id: requestId++,
      method: call.method,
      params: call.params ?? []
    }));

    const response = await fetch(this.url, {
      method: "POST",
      headers: {
        "content-type": "application/json"
      },
      body: JSON.stringify(payload)
    });

    const text = await response.text();

    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      return {
        supported: false,
        request: payload,
        responses: [
          {
            id: -1,
            ok: false,
            error: {
              code: response.status,
              message: "Non-JSON batch response",
              data: text,
              raw: text
            },
            raw: text
          }
        ],
        raw: text
      };
    }

    if (!Array.isArray(parsed)) {
      let error: RpcErrorShape | undefined;
      if (parsed && typeof parsed === "object" && "error" in parsed) {
        const envelope = parsed as { error?: unknown };
        if (envelope.error !== null && envelope.error !== undefined) {
          error = toRpcError(envelope.error, "Batch request failed");
        }
      }

      return {
        supported: false,
        request: payload,
        responses: [
          {
            id: -1,
            ok: false,
            error: error ?? {
              code: null,
              message: "Batch request unsupported",
              data: parsed,
              raw: parsed
            },
            raw: parsed
          }
        ],
        raw: parsed
      };
    }

    const idToMethod = new Map<number, string>();
    for (const req of payload) {
      idToMethod.set(req.id, req.method);
    }

    const responses = parsed.map((item) => {
      if (!item || typeof item !== "object") {
        return {
          id: -1,
          ok: false,
          error: {
            code: null,
            message: "Malformed batch item",
            data: item,
            raw: item
          },
          raw: item
        };
      }

      const envelope = item as { id?: unknown; result?: unknown; error?: unknown };
      const id = typeof envelope.id === "number" ? envelope.id : -1;
      const hasResult = "result" in envelope;
      const hasError = "error" in envelope && envelope.error !== null && envelope.error !== undefined;

      if (hasError) {
        return {
          id,
          method: idToMethod.get(id),
          ok: false,
          error: toRpcError(envelope.error, "Batch item failed"),
          raw: item
        };
      }

      if (hasResult) {
        return {
          id,
          method: idToMethod.get(id),
          ok: true,
          result: envelope.result,
          raw: item
        };
      }

      return {
        id,
        method: idToMethod.get(id),
        ok: false,
        error: {
          code: null,
          message: "Malformed batch item",
          data: item,
          raw: item
        },
        raw: item
      };
    });

    responses.sort((a, b) => a.id - b.id);

    return {
      supported: true,
      request: payload,
      responses,
      raw: parsed
    };
  }
}

export async function waitForRpcReady(url: string, timeoutMs = 15_000): Promise<void> {
  const client = new JsonRpcClient(url);
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    try {
      const chainId = await client.call("eth_chainId", []);
      if (chainId.ok && decodeHexQuantity(chainId.result) !== null) {
        return;
      }
    } catch {
      // Retry.
    }
    await delay(300);
  }

  throw new Error(`RPC endpoint did not become ready within ${timeoutMs}ms: ${url}`);
}
