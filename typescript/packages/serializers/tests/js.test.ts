import { describe, expect, it } from "vitest";
import { JsSerializer } from "../src";
import schema from "./fuzz-input-schema.json";

const js = new JsSerializer(schema);

const byteVecCases: Array<[string, any]> = [
  ["Base58", { address: "gFqoeNwi4sf1M" }],
  ["Hex", "0x1717171717171717171717171717171717171717171717171717171717171717"],
  // handles non 0x prefix
  ["Hex", "17171717171717171717171717171717171717171700"],
  // purposely insert weird whitespace
  ["Decimal", "[2,4, 5,1, 0, 2]"],
];

// TODO: get 32byte values
const byteArrayCases: Array<[string, any]> = [
  ["Hex", "0x1717171717171717171717171717171717171717171717171717171717171717"],
  // handles non 0x prefix
  ["Hex", "1717171717171717171717171717171717171717171717171717171717171717"],
];

describe("js", () => {
  describe("ByteVec byteDisplay", () => {
    it.each(byteVecCases.map(([format, input]) => ({ format, input })))(
      "should serialize $format: $input",
      ({ format, input }) => {
        const data = { ByteVec: { [format]: input } };
        expect(() => js.serialize(data, 0)).not.toThrow();
      },
    );
  });
  describe("ByteArray byteDisplay", () => {
    it.each(byteArrayCases.map(([format, input]) => ({ format, input })))(
      "should serialize $format: $input",
      ({ format, input }) => {
        const data = { ByteArray: { [format]: input } };
        expect(() => js.serialize(data, 0)).not.toThrow();
      },
    );
  });
});
