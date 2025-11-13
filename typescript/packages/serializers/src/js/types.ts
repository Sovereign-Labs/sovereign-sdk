export interface Schema {
  types: Ty[];
  root_type_indices: number[];
  chain_data: ChainData;
  templates: TransactionTemplateSet[];
  serde_metadata: ContainerSerdeMetadata[];
}

export interface ChainData {
  chain_id: number;
  chain_name: string;
}

export type TransactionTemplateSet = Record<string, any>;

export interface ContainerSerdeMetadata {
  name: string;
  fields_or_variants: FieldOrVariantSerdeMetadata[];
}

export interface FieldOrVariantSerdeMetadata {
  name: string;
}

export type Ty =
  | { Enum: EnumType }
  | { Struct: StructType }
  | { Tuple: TupleType }
  | { Option: { value: Link } }
  | { Integer: [IntegerType, IntegerDisplay] }
  | { ByteArray: { len: number; display: ByteDisplay } }
  | "Float32"
  | "Float64"
  | "String"
  | "Boolean"
  | { Skip: { len: number } }
  | { ByteVec: { display: ByteDisplay } }
  | { Array: { len: number; value: Link } }
  | { Vec: { value: Link } }
  | { Map: { key: Link; value: Link } };

export interface EnumType {
  type_name: string;
  variants: EnumVariant[];
  hide_tag: boolean;
}

export interface EnumVariant {
  name: string;
  discriminant: number;
  template: string | null;
  value: Link | null;
}

export interface StructType {
  type_name: string;
  template: string | null;
  peekable: boolean;
  fields: NamedField[];
}

export interface TupleType {
  template: string | null;
  peekable: boolean;
  fields: UnnamedField[];
}

export interface NamedField {
  display_name: string;
  silent: boolean;
  value: Link;
  doc: string;
}

export interface UnnamedField {
  value: Link;
  silent: boolean;
  doc: string;
}

export type Link =
  | { ByIndex: number }
  | { Immediate: Primitive }
  | "Placeholder"
  | { IndexedPlaceholder: number };

export type Primitive =
  | { Integer: [IntegerType, IntegerDisplay] }
  | { ByteArray: { len: number; display: ByteDisplay } }
  | { ByteVec: { display: ByteDisplay } }
  | "Float32"
  | "Float64"
  | "String"
  | "Boolean"
  | { Skip: { len: number } };

export type IntegerType =
  | "i8"
  | "i16"
  | "i32"
  | "i64"
  | "i128"
  | "u8"
  | "u16"
  | "u32"
  | "u64"
  | "u128";

export type IntegerDisplay =
  | "Hex"
  | "Decimal"
  | { FixedPoint: FixedPointDisplay };

export type FixedPointDisplay =
  | { Decimals: number }
  | { FromSiblingField: { field_index: number; byte_offset: number } };

export type ByteDisplay =
  | "Hex"
  | "Decimal"
  | "Base58"
  | { Bech32: { prefix: string } }
  | { Bech32m: { prefix: string } };

export class SerializationError extends Error {
  constructor(message: string, public readonly kind: string) {
    super(message);
    this.name = "SerializationError";
  }
}
