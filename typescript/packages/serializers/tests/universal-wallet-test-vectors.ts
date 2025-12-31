import * as fs from "node:fs";
import * as path from "node:path";
import { bytesToHex } from "@sovereign-sdk/utils";
import { describe, expect, test } from "vitest";
import { JsSerializer } from "../src";

interface TestVector {
  name: string;
  input: Record<string, unknown>;
  output: string;
  schema: string;
}

const vectorDir = path.join(import.meta.dirname, "vectors", "universal-wallet");

function loadUniversalWalletVectors(): TestVector[] {
  const result = [];
  const files = fs.readdirSync(vectorDir);

  for (const file of files) {
    const content = fs.readFileSync(path.join(vectorDir, file), "utf8");
    const test = JSON.parse(content);

    result.push({
      name: file,
      input: JSON.parse(test.input),
      output: test.output,
      schema: test.schema,
    });
  }

  return result;
}

describe("universal wallet", () => {
  test.each(loadUniversalWalletVectors())("$name", ({ name, ...vector }) => {
    const js = new JsSerializer(JSON.parse(vector.schema));
    const actual = bytesToHex(js.serialize(vector.input, 0));
    expect(actual).toEqual(vector.output);
  });
});
