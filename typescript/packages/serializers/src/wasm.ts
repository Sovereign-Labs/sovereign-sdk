import { Schema } from "@sovereign-sdk/universal-wallet-wasm";
import { Serializer } from "./serializer";

export * from "./serializer";

export class WasmSerializer extends Serializer {
  protected jsonToBorsh(input: unknown, index: number): Uint8Array {
    const jsonString =
      typeof input === "string" ? input : JSON.stringify(input);
    const schema = Schema.fromJSON(JSON.stringify(this._schema));
    return schema.jsonToBorsh(index, jsonString);
  }
}
