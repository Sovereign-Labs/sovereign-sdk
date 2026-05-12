/**
 * A rollup schema is a description of the types that are utilized in the rollup.
 * It is used to serialize and deserialize the types.
 * As well as display them in a human-readable way that is verified by the rollup.
 */
// biome-ignore lint/suspicious/noExplicitAny: types arent used
export type RollupSchema = Record<string, any>;

export enum KnownTypeId {
  /** The type id of the transaction. */
  Transaction = 0,
  /** The type id of the unsigned transaction. */
  UnsignedTransaction = 1,
  /** The type id of the runtime call. */
  RuntimeCall = 2,
}

/**
 * A serializer is used to serialize rollup types to Borsh bytes.
 *
 * @example
 * ```typescript
 * const serializer = createSerializer(yourSchema);
 * const runtimeCall = {
 *   value_setter: {
 *     set_value: {
 *       value: 100,
 *       gas: null
 *     },
 *   },
 * };
 * const borshBytes = serializer.serializeRuntimeCall(runtimeCall);
 * ```
 */
export abstract class Serializer {
  protected _schema: RollupSchema;

  constructor(schema: RollupSchema) {
    this._schema = schema;
  }

  protected abstract jsonToBorsh(input: unknown, index: number): Uint8Array;

  private lookupKnownTypeIndex(id: KnownTypeId): number {
    return this._schema.root_type_indices[id];
  }

  /**
   * Serialize an input to Borsh bytes.
   *
   * Treats `Uint8Array` as a plain array of numbers as this is the format used by universal-wallet
   * to represent Borsh bytes.
   *
   * @param input - The input to serialize.
   * @param index - The index of the type in the schema to serialize.
   * @returns The serialized Borsh bytes.
   */
  serialize(input: unknown, index: number): Uint8Array {
    // Allows supplying `Uint8Array` for byte array fields in the schema
    const obj = convertUint8ArraysToArrays(input);
    return this.jsonToBorsh(obj, index);
  }

  /**
   * Serialize a runtime call to Borsh bytes.
   *
   * @param input - The runtime call to serialize.
   * @returns The serialized Borsh bytes.
   */
  serializeRuntimeCall(input: unknown): Uint8Array {
    return this.serialize(
      input,
      this.lookupKnownTypeIndex(KnownTypeId.RuntimeCall),
    );
  }

  /**
   * Serialize an unsigned transaction to Borsh bytes.
   *
   * @param input - The unsigned transaction to serialize.
   * @returns The serialized Borsh bytes.
   */
  serializeUnsignedTx(input: unknown): Uint8Array {
    return this.serialize(
      input,
      this.lookupKnownTypeIndex(KnownTypeId.UnsignedTransaction),
    );
  }

  /**
   * Serialize a transaction to Borsh bytes.
   *
   * @param input - The transaction to serialize.
   * @returns The serialized Borsh bytes.
   */
  serializeTx(input: unknown): Uint8Array {
    return this.serialize(
      input,
      this.lookupKnownTypeIndex(KnownTypeId.Transaction),
    );
  }

  /** Returns the Schema JSON used by the serializer. */
  get schema(): RollupSchema {
    return { ...this._schema };
  }
}

/**
 * Performs deep traversal of the provided object and replaces all
 * `Uint8Array` instances with `Array`s to make them compatible
 * with `universal-wallet` serialization schemas.
 *
 * We do this for `Uint8Array` so it can be passed for `ByteVec` / `ByteArray` fields.
 */
export function convertUint8ArraysToArrays<T>(obj: T): T {
  if (obj === null || obj === undefined) {
    return obj;
  }

  if (obj instanceof Uint8Array) {
    return Array.from(obj) as unknown as T;
  }

  if (Array.isArray(obj)) {
    return obj.map((item) => convertUint8ArraysToArrays(item)) as unknown as T;
  }

  if (typeof obj === "object" && obj.constructor === Object) {
    const result: Record<string, unknown> = {};
    for (const key in obj) {
      // biome-ignore lint/suspicious/noPrototypeBuiltins: hasOwn not available
      if (obj.hasOwnProperty(key)) {
        result[key] = convertUint8ArraysToArrays(obj[key]);
      }
    }
    return result as T;
  }

  return obj;
}
