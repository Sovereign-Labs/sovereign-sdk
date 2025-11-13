import { BorshWriter } from "./borsh-writer";
import * as byteDisplay from "./byte-display";
import {
  type ContainerSerdeMetadata,
  type EnumType,
  type IntegerType,
  type Link,
  type Primitive,
  type Schema,
  SerializationError,
  type StructType,
  type TupleType,
  type Ty,
} from "./types";

interface Context {
  value: any;
  currentLink: Link;
}

export class JsonToBorshConverter {
  private schema: Schema;
  private writer: BorshWriter;

  constructor(schema: Schema) {
    this.schema = schema;
    this.writer = new BorshWriter();
  }

  static convertToBytes(
    schema: Schema,
    typeIndex: number,
    input: unknown
  ): Uint8Array {
    const converter = new JsonToBorshConverter(schema);
    return converter.convert(typeIndex, input);
  }

  static convertToHex(
    schema: Schema,
    typeIndex: number,
    input: unknown
  ): string {
    const bytes = JsonToBorshConverter.convertToBytes(schema, typeIndex, input);
    return Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }

  private convert(typeIndex: number, input: unknown): Uint8Array {
    const ty = this.schema.types[typeIndex];
    if (!ty) {
      throw new SerializationError(
        `Invalid type index: ${typeIndex}`,
        "InvalidIndex"
      );
    }

    const context: Context = {
      value: input,
      currentLink: { ByIndex: typeIndex },
    };

    this.visitType(ty, context);
    return this.writer.toUint8Array();
  }

  private visitType(ty: Ty, context: Context): void {
    if (typeof ty === "string") {
      this.visitPrimitiveString(ty, context);
    } else if ("Enum" in ty) {
      this.visitEnum(ty.Enum, context);
    } else if ("Struct" in ty) {
      this.visitStruct(ty.Struct, context);
    } else if ("Tuple" in ty) {
      this.visitTuple(ty.Tuple, context);
    } else if ("Option" in ty) {
      this.visitOption(ty.Option.value, context);
    } else if ("Integer" in ty) {
      this.visitInteger(ty.Integer[0], ty.Integer[1], context);
    } else if ("ByteArray" in ty) {
      this.visitByteArray(ty.ByteArray.len, ty.ByteArray.display, context);
    } else if ("ByteVec" in ty) {
      this.visitByteVec(ty.ByteVec.display, context);
    } else if ("Array" in ty) {
      this.visitArray(ty.Array.len, ty.Array.value, context);
    } else if ("Vec" in ty) {
      this.visitVec(ty.Vec.value, context);
    } else if ("Map" in ty) {
      this.visitMap(ty.Map.key, ty.Map.value, context);
    } else if ("Skip" in ty) {
      // Skip types don't write anything
    } else {
      throw new SerializationError(
        `Unknown type: ${JSON.stringify(ty)}`,
        "InvalidType"
      );
    }
  }

  private visitPrimitiveString(ty: string, context: Context): void {
    switch (ty) {
      case "Float32":
        this.visitFloat32(context);
        break;
      case "Float64":
        this.visitFloat64(context);
        break;
      case "String":
        this.visitString(context);
        break;
      case "Boolean":
        this.visitBoolean(context);
        break;
      default:
        throw new SerializationError(
          `Unknown primitive type: ${ty}`,
          "InvalidType"
        );
    }
  }

  private visitEnum(enumType: EnumType, context: Context): void {
    let discriminant: string;
    let innerValue: any = null;

    if (typeof context.value === "string") {
      discriminant = context.value;
    } else if (typeof context.value === "object" && context.value !== null) {
      const keys = Object.keys(context.value);
      if (keys.length !== 1) {
        throw new SerializationError(
          `Invalid enum encoding: expected single variant, found object with ${keys.length} JSON properties`,
          "MalformedEnum"
        );
      }
      discriminant = keys[0];
      innerValue = context.value[discriminant];
    } else {
      throw new SerializationError(
        `Expected enum ${
          enumType.type_name
        }, encountered invalid JSON value ${JSON.stringify(context.value)}`,
        "InvalidType"
      );
    }

    // Get serde metadata for this enum
    const metadata = this.getSerdeMetadata(context.currentLink);
    if (!metadata) {
      throw new SerializationError(
        `Type ${enumType.type_name} did not have serde metadata present in the schema`,
        "MissingMetadata"
      );
    }

    // Find the variant
    const variantIndex = metadata.fields_or_variants.findIndex(
      (v) => v.name === discriminant
    );
    if (variantIndex === -1) {
      throw new SerializationError(
        `Invalid discriminant \`${discriminant}\` for ${enumType.type_name}`,
        "InvalidDiscriminant"
      );
    }

    const variant = enumType.variants[variantIndex];
    if (!variant) {
      throw new SerializationError(
        `Invalid discriminant \`${discriminant}\` for ${enumType.type_name}`,
        "InvalidDiscriminant"
      );
    }

    // Write discriminant
    this.writer.writeU8(variant.discriminant);

    // Handle variant value
    if (variant.value) {
      const innerType = this.resolveLink(variant.value);
      const innerContext: Context = {
        value: innerValue,
        currentLink: variant.value,
      };
      this.visitType(innerType, innerContext);
    } else if (innerValue !== null && innerValue !== undefined) {
      throw new SerializationError(
        `The JSON contained an unexpected extra value: ${JSON.stringify(
          innerValue
        )}`,
        "UnusedInput"
      );
    }
  }

  private visitStruct(structType: StructType, context: Context): void {
    if (typeof context.value !== "object" || context.value === null) {
      throw new SerializationError(
        `Expected ${
          structType.type_name
        } struct, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    const jsonFields = { ...context.value };
    const metadata = this.getSerdeMetadata(context.currentLink);
    if (!metadata) {
      throw new SerializationError(
        `Type ${structType.type_name} did not have serde metadata present in the schema`,
        "MissingMetadata"
      );
    }

    for (let i = 0; i < structType.fields.length; i++) {
      const field = structType.fields[i];
      const fieldSerde = metadata.fields_or_variants[i];

      if (!(fieldSerde.name in jsonFields)) {
        throw new SerializationError(
          `Expected type or field ${structType.type_name}.${field.display_name}, but it was not present`,
          "MissingType"
        );
      }

      const jsonValue = jsonFields[fieldSerde.name];
      delete jsonFields[fieldSerde.name];

      const innerType = this.resolveLink(field.value);
      const fieldContext: Context = {
        value: jsonValue,
        currentLink: field.value,
      };
      this.visitType(innerType, fieldContext);
    }

    const remainingKeys = Object.keys(jsonFields);
    if (remainingKeys.length > 0) {
      throw new SerializationError(
        `The JSON contained an unexpected extra value: ${JSON.stringify(
          jsonFields
        )}`,
        "UnusedInput"
      );
    }
  }

  private visitTuple(tupleType: TupleType, context: Context): void {
    if (tupleType.fields.length === 1) {
      // Trivial tuples aren't wrapped in JSON; forward the value directly
      const field = tupleType.fields[0];
      const innerType = this.resolveLink(field.value);
      const innerContext: Context = {
        value: context.value,
        currentLink: field.value,
      };
      this.visitType(innerType, innerContext);
    } else {
      if (!Array.isArray(context.value)) {
        throw new SerializationError(
          `Expected array, encountered invalid JSON value ${JSON.stringify(
            context.value
          )}`,
          "InvalidType"
        );
      }

      if (context.value.length !== tupleType.fields.length) {
        throw new SerializationError(
          `Expected an array of size ${tupleType.fields.length}, but only found ${context.value.length} elements in the JSON`,
          "WrongArrayLength"
        );
      }

      for (let i = 0; i < tupleType.fields.length; i++) {
        const field = tupleType.fields[i];
        const innerType = this.resolveLink(field.value);
        const fieldContext: Context = {
          value: context.value[i],
          currentLink: field.value,
        };
        this.visitType(innerType, fieldContext);
      }
    }
  }

  private visitOption(valueLink: Link, context: Context): void {
    if (context.value === null || context.value === undefined) {
      this.writer.writeU8(0);
    } else {
      this.writer.writeU8(1);
      const innerType = this.resolveLink(valueLink);
      const innerContext: Context = {
        value: context.value,
        currentLink: valueLink,
      };
      this.visitType(innerType, innerContext);
    }
  }

  private visitInteger(
    integerType: IntegerType,
    _display: any,
    context: Context
  ): void {
    let value: number | bigint;

    if (typeof context.value === "number") {
      value = context.value;
    } else if (typeof context.value === "bigint") {
      value = context.value;
    } else if (typeof context.value === "string") {
      try {
        if (integerType.includes("128") || integerType.includes("64")) {
          value = BigInt(context.value);
        } else {
          const parsed = Number.parseInt(context.value, 10);
          if (isNaN(parsed)) {
            throw new Error("NaN");
          }
          value = parsed;
        }
      } catch {
        throw new SerializationError(
          `Expected ${integerType}, encountered invalid JSON value ${JSON.stringify(
            context.value
          )}`,
          "InvalidType"
        );
      }
    } else {
      throw new SerializationError(
        `Expected ${integerType}, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    switch (integerType) {
      case "i8":
        this.writer.writeI8(value as number);
        break;
      case "i16":
        this.writer.writeI16(value as number);
        break;
      case "i32":
        this.writer.writeI32(value as number);
        break;
      case "i64":
        this.writer.writeI64(value);
        break;
      case "i128":
        this.writer.writeI128(value);
        break;
      case "u8":
        this.writer.writeU8(value as number);
        break;
      case "u16":
        this.writer.writeU16(value as number);
        break;
      case "u32":
        this.writer.writeU32(value as number);
        break;
      case "u64":
        this.writer.writeU64(value);
        break;
      case "u128":
        this.writer.writeU128(value);
        break;
      default:
        throw new SerializationError(
          `Unknown integer type: ${integerType}`,
          "InvalidType"
        );
    }
  }

  private visitFloat32(context: Context): void {
    let value: number;

    if (typeof context.value === "number") {
      value = context.value;
    } else if (typeof context.value === "string") {
      if (context.value.startsWith("0x")) {
        value = this.hexToFloat(context.value, 32);
      } else {
        value = Number.parseFloat(context.value);
        if (isNaN(value)) {
          throw new SerializationError(
            `Expected f32, encountered invalid JSON value ${JSON.stringify(
              context.value
            )}`,
            "InvalidType"
          );
        }
      }
    } else {
      throw new SerializationError(
        `Expected f32, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    // Check if value is finite as per Rust implementation
    if (!Number.isFinite(value)) {
      throw new SerializationError(
        `Expected f32, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    this.writer.writeF32(value);
  }

  private visitFloat64(context: Context): void {
    let value: number;

    if (typeof context.value === "number") {
      value = context.value;
    } else if (typeof context.value === "string") {
      if (context.value.startsWith("0x")) {
        value = this.hexToFloat(context.value, 64);
      } else {
        value = Number.parseFloat(context.value);
        if (isNaN(value)) {
          throw new SerializationError(
            `Expected f64, encountered invalid JSON value ${JSON.stringify(
              context.value
            )}`,
            "InvalidType"
          );
        }
      }
    } else {
      throw new SerializationError(
        `Expected f64, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    this.writer.writeF64(value);
  }

  private visitString(context: Context): void {
    if (typeof context.value !== "string") {
      throw new SerializationError(
        `Expected String, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }
    this.writer.writeString(context.value);
  }

  private visitBoolean(context: Context): void {
    let value: boolean;

    if (typeof context.value === "boolean") {
      value = context.value;
    } else if (typeof context.value === "string") {
      if (context.value === "true") {
        value = true;
      } else if (context.value === "false") {
        value = false;
      } else {
        throw new SerializationError(
          `Expected bool, encountered invalid JSON value ${JSON.stringify(
            context.value
          )}`,
          "InvalidType"
        );
      }
    } else {
      throw new SerializationError(
        `Expected bool, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    this.writer.writeBool(value);
  }

  private visitByteArray(len: number, display: any, context: Context): void {
    let bytes: number[];

    if (Array.isArray(context.value)) {
      bytes = context.value.map((v, i) => {
        if (typeof v !== "number" || v < 0 || v > 255 || !Number.isInteger(v)) {
          throw new SerializationError(
            `Expected byte, encountered invalid JSON value ${JSON.stringify(
              v
            )}`,
            "InvalidType"
          );
        }
        return v;
      });
    } else if (typeof context.value === "string") {
      // Use the display from context.currentLink if it's an Immediate link,
      // otherwise use the display parameter passed directly
      const actualDisplay = 
        typeof context.currentLink === "object" && 
        "Immediate" in context.currentLink && 
        typeof context.currentLink.Immediate === "object" &&
        "ByteArray" in context.currentLink.Immediate
          ? context.currentLink.Immediate.ByteArray.display
          : display;
      bytes = byteDisplay.parse(
        actualDisplay,
        context.value
      );
    } else {
      throw new SerializationError(
        `Expected byte array, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    if (bytes.length !== len) {
      throw new SerializationError(
        `Expected an array of size ${len}, but only found ${bytes.length} elements in the JSON`,
        "WrongArrayLength"
      );
    }

    for (const byte of bytes) {
      this.writer.writeU8(byte);
    }
  }

  private visitByteVec(display: any, context: Context): void {
    let bytes: number[];

    if (Array.isArray(context.value)) {
      bytes = context.value.map((v) => {
        if (typeof v !== "number" || v < 0 || v > 255 || !Number.isInteger(v)) {
          throw new SerializationError(
            `Expected byte, encountered invalid JSON value ${JSON.stringify(
              v
            )}`,
            "InvalidType"
          );
        }
        return v;
      });
    } else if (typeof context.value === "string") {
      // Use the display from context.currentLink if it's an Immediate link,
      // otherwise use the display parameter passed directly
      const actualDisplay = 
        typeof context.currentLink === "object" && 
        "Immediate" in context.currentLink && 
        typeof context.currentLink.Immediate === "object" &&
        "ByteVec" in context.currentLink.Immediate
          ? context.currentLink.Immediate.ByteVec.display
          : display;
      bytes = byteDisplay.parse(
        actualDisplay,
        context.value
      );
    } else {
      throw new SerializationError(
        `Expected byte vector, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    this.writer.writeU32(bytes.length);
    for (const byte of bytes) {
      this.writer.writeU8(byte);
    }
  }

  private visitArray(len: number, valueLink: Link, context: Context): void {
    if (!Array.isArray(context.value)) {
      throw new SerializationError(
        `Expected array, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    if (context.value.length !== len) {
      throw new SerializationError(
        `Expected an array of size ${len}, but only found ${context.value.length} elements in the JSON`,
        "WrongArrayLength"
      );
    }

    const innerType = this.resolveLink(valueLink);
    for (const item of context.value) {
      const itemContext: Context = {
        value: item,
        currentLink: valueLink,
      };
      this.visitType(innerType, itemContext);
    }
  }

  private visitVec(valueLink: Link, context: Context): void {
    if (!Array.isArray(context.value)) {
      throw new SerializationError(
        `Expected vector, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    if (context.value.length > 0xffffffff) {
      throw new SerializationError(
        `Only array sizes that fit into u32 are supported; input contained size ${context.value.length}`,
        "InvalidVecLength"
      );
    }

    this.writer.writeU32(context.value.length);
    const innerType = this.resolveLink(valueLink);
    for (const item of context.value) {
      const itemContext: Context = {
        value: item,
        currentLink: valueLink,
      };
      this.visitType(innerType, itemContext);
    }
  }

  private visitMap(keyLink: Link, valueLink: Link, context: Context): void {
    if (
      typeof context.value !== "object" ||
      context.value === null ||
      Array.isArray(context.value)
    ) {
      throw new SerializationError(
        `Expected map, encountered invalid JSON value ${JSON.stringify(
          context.value
        )}`,
        "InvalidType"
      );
    }

    // Sort entries by key to ensure deterministic ordering (matching Rust behavior)
    const entries = Object.entries(context.value).sort(([a], [b]) =>
      a.localeCompare(b)
    );
    if (entries.length > 0xffffffff) {
      throw new SerializationError(
        `Only array sizes that fit into u32 are supported; input contained size ${entries.length}`,
        "InvalidVecLength"
      );
    }

    this.writer.writeU32(entries.length);
    const keyType = this.resolveLink(keyLink);
    const valueType = this.resolveLink(valueLink);

    for (const [key, value] of entries) {
      // JSON coerces all map keys to string. Handle numeric keys specially.
      let keyValue: any = key;
      if (this.isNumericType(keyType)) {
        try {
          keyValue = Number.parseFloat(key);
        } catch {
          throw new SerializationError(`Invalid JSON: ${key}`, "Json");
        }
      }

      const keyContext: Context = {
        value: keyValue,
        currentLink: keyLink,
      };
      this.visitType(keyType, keyContext);

      const valueContext: Context = {
        value: value,
        currentLink: valueLink,
      };
      this.visitType(valueType, valueContext);
    }
  }

  private isNumericType(ty: Ty): boolean {
    if (typeof ty === "string") {
      return ty === "Float32" || ty === "Float64";
    }
    return "Integer" in ty;
  }

  private hexToFloat(hexString: string, precision = 32): number {
    const hex = hexString.replace(/^0x/, "");
    const bytes = hex.match(/.{2}/g)!.map((b) => parseInt(b, 16));
    const view = new DataView(new Uint8Array(bytes).buffer);
    return precision === 64
      ? view.getFloat64(0, true)
      : view.getFloat32(0, true);
  }

  private resolveLink(link: Link): Ty {
    if (typeof link === "string") {
      throw new SerializationError(
        `Unresolved placeholder link: ${link}`,
        "UnresolvedType"
      );
    }

    if ("ByIndex" in link) {
      const ty = this.schema.types[link.ByIndex];
      if (!ty) {
        throw new SerializationError(
          `Invalid type index: ${link.ByIndex}`,
          "UnresolvedType"
        );
      }
      return ty;
    } else if ("Immediate" in link) {
      return this.primitiveToTy(link.Immediate);
    } else {
      throw new SerializationError(
        `Unresolved placeholder link: ${JSON.stringify(link)}`,
        "UnresolvedType"
      );
    }
  }

  private primitiveToTy(primitive: Primitive): Ty {
    if (typeof primitive === "string") {
      return primitive;
    } else if ("Integer" in primitive) {
      return { Integer: primitive.Integer };
    } else if ("ByteArray" in primitive) {
      return { ByteArray: primitive.ByteArray };
    } else if ("ByteVec" in primitive) {
      return { ByteVec: primitive.ByteVec };
    } else if ("Skip" in primitive) {
      return { Skip: primitive.Skip };
    } else {
      throw new SerializationError(
        `Unknown primitive: ${JSON.stringify(primitive)}`,
        "InvalidType"
      );
    }
  }

  private getSerdeMetadata(link: Link): ContainerSerdeMetadata | null {
    if (
      typeof link === "string" ||
      "Immediate" in link ||
      "IndexedPlaceholder" in link
    ) {
      return null;
    }

    if ("ByIndex" in link) {
      return this.schema.serde_metadata[link.ByIndex] || null;
    }

    return null;
  }
}

export function jsonToBorsh(
  schema: Schema,
  typeIndex: number,
  input: unknown
): Uint8Array {
  return JsonToBorshConverter.convertToBytes(schema, typeIndex, input);
}
