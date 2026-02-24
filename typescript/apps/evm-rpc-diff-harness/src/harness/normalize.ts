import { isHexQuantity } from "./rpc";

export interface DiffEntry {
  path: string;
  expected: unknown;
  actual: unknown;
  reason: string;
}

const VOLATILE_KEYS = new Set([
  "hash",
  "blockHash",
  "blockNumber",
  "transactionHash",
  "transactionIndex",
  "logIndex",
  "timestamp"
]);

// Returns value unchanged if it's not a valid hex string (pass-through by design).
function normalizeHex(value: string): string {
  if (!/^0x[0-9a-fA-F]*$/.test(value)) {
    return value;
  }
  return `0x${value.slice(2).toLowerCase()}`;
}

export function normalizeForComparison(value: unknown): unknown {
  if (typeof value === "bigint") {
    return value.toString();
  }

  if (typeof value === "string") {
    return normalizeHex(value);
  }

  if (Array.isArray(value)) {
    return value.map((entry) => normalizeForComparison(entry));
  }

  if (value && typeof value === "object") {
    const obj = value as Record<string, unknown>;
    const normalized: Record<string, unknown> = {};

    for (const key of Object.keys(obj).sort()) {
      if (VOLATILE_KEYS.has(key)) {
        continue;
      }
      normalized[key] = normalizeForComparison(obj[key]);
    }

    return normalized;
  }

  return value;
}

export function minimalJsonDiff(expected: unknown, actual: unknown, maxEntries = 30): DiffEntry[] {
  const diffs: DiffEntry[] = [];

  function walk(path: string, left: unknown, right: unknown): void {
    if (diffs.length >= maxEntries) {
      return;
    }

    if (left === right) {
      return;
    }

    const leftType = Array.isArray(left) ? "array" : typeof left;
    const rightType = Array.isArray(right) ? "array" : typeof right;

    if (leftType !== rightType) {
      diffs.push({
        path,
        expected: left,
        actual: right,
        reason: `Type mismatch (${leftType} vs ${rightType})`
      });
      return;
    }

    if (Array.isArray(left) && Array.isArray(right)) {
      if (left.length !== right.length) {
        diffs.push({
          path,
          expected: left.length,
          actual: right.length,
          reason: "Array length mismatch"
        });
      }

      const minLen = Math.min(left.length, right.length);
      for (let i = 0; i < minLen; i += 1) {
        walk(`${path}[${i}]`, left[i], right[i]);
      }
      return;
    }

    if (left && typeof left === "object" && right && typeof right === "object") {
      const leftObj = left as Record<string, unknown>;
      const rightObj = right as Record<string, unknown>;
      const keys = new Set([...Object.keys(leftObj), ...Object.keys(rightObj)]);

      for (const key of Array.from(keys).sort()) {
        if (!(key in leftObj)) {
          diffs.push({
            path: `${path}.${key}`,
            expected: undefined,
            actual: rightObj[key],
            reason: "Missing expected key"
          });
          continue;
        }

        if (!(key in rightObj)) {
          diffs.push({
            path: `${path}.${key}`,
            expected: leftObj[key],
            actual: undefined,
            reason: "Missing actual key"
          });
          continue;
        }

        walk(`${path}.${key}`, leftObj[key], rightObj[key]);
      }
      return;
    }

    diffs.push({
      path,
      expected: left,
      actual: right,
      reason: "Value mismatch"
    });
  }

  walk("$", normalizeForComparison(expected), normalizeForComparison(actual));
  return diffs;
}

export function deepEqualNormalized(left: unknown, right: unknown): boolean {
  return JSON.stringify(normalizeForComparison(left)) === JSON.stringify(normalizeForComparison(right));
}

function isHex(value: unknown, bytes?: number): boolean {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]+$/.test(value)) {
    return false;
  }

  if (bytes !== undefined) {
    return value.length === 2 + bytes * 2;
  }

  return true;
}

export interface ShapeReport {
  ok: boolean;
  issues: string[];
}

export function validateBlockShape(block: unknown, fullTransactions: boolean): ShapeReport {
  const issues: string[] = [];

  if (!block || typeof block !== "object") {
    return { ok: false, issues: ["Block response is not an object"] };
  }

  const entry = block as Record<string, unknown>;

  if (!isHexQuantity(entry.number)) {
    issues.push("number is missing or not a hex quantity");
  }

  if (!isHex(entry.hash, 32)) {
    issues.push("hash is missing or not a 32-byte hex string");
  }

  if (!isHex(entry.parentHash, 32)) {
    issues.push("parentHash is missing or not a 32-byte hex string");
  }

  if (!isHexQuantity(entry.timestamp)) {
    issues.push("timestamp is missing or not a hex quantity");
  }

  if (!Array.isArray(entry.transactions)) {
    issues.push("transactions is not an array");
  } else if (fullTransactions) {
    for (const tx of entry.transactions) {
      if (!tx || typeof tx !== "object") {
        issues.push("full transaction entry is not an object");
        break;
      }
      const txObj = tx as Record<string, unknown>;
      if (!isHex(txObj.hash, 32)) {
        issues.push("full transaction entry missing hash");
        break;
      }
      if (!isHexQuantity(txObj.nonce)) {
        issues.push("full transaction entry missing nonce");
        break;
      }
      if (!isHex(txObj.from, 20)) {
        issues.push("full transaction entry missing from");
        break;
      }
    }
  } else {
    for (const tx of entry.transactions) {
      if (!isHex(tx, 32)) {
        issues.push("transaction hash entry is not a 32-byte hex string");
        break;
      }
    }
  }

  if (entry.baseFeePerGas !== undefined && !isHexQuantity(entry.baseFeePerGas)) {
    issues.push("baseFeePerGas is present but not a hex quantity");
  }

  return {
    ok: issues.length === 0,
    issues
  };
}

export function summarizeErrorShape(error: unknown): { hasMessage: boolean; hasData: boolean; code: number | null } {
  if (!error || typeof error !== "object") {
    return { hasMessage: false, hasData: false, code: null };
  }

  const obj = error as { code?: unknown; message?: unknown; data?: unknown };
  return {
    hasMessage: typeof obj.message === "string",
    hasData: obj.data !== undefined,
    code: typeof obj.code === "number" ? obj.code : null
  };
}
