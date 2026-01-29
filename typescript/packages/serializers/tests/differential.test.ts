import { describe, expect, it } from "vitest";
import demoRollupSchema from "../../__fixtures__/demo-rollup-schema.json";
import { JsSerializer } from "../src";
import { WasmSerializer } from "../src/wasm";
import demoRollupVectors from "./vectors/demo-rollup";

describe("differential", () => {
  describe("demo rollup", () => {
    it.each(demoRollupVectors.calls.map((call, index) => ({ index, call })))(
      "should serialize runtime call $index",
      ({ call }) => {
        const js = new JsSerializer(demoRollupSchema);
        const wasm = new WasmSerializer(demoRollupSchema);
        expect(js.serializeRuntimeCall(call)).toEqual(
          wasm.serializeRuntimeCall(call),
        );
      },
    );
  });
});
