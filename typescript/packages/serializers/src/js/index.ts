import { Serializer } from "../serializer";
import { jsonToBorsh } from "./json-to-borsh";
import type { Schema } from "./types";

export class JsSerializer extends Serializer {
  protected jsonToBorsh(input: unknown, index: number): Uint8Array {
    return jsonToBorsh(this._schema as Schema, index, input);
  }
}
