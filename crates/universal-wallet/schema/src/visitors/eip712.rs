use std::collections::HashMap;

use alloy_dyn_abi::{Eip712Types, PropertyDef};
use serde_json::Map;
use thiserror::Error;

use crate::schema::Primitive;
use crate::ty::visitor::{ResolutionError, TypeResolver, TypeVisitor};
use crate::ty::{
    byte_display, ByteDisplay, Enum, IntegerDisplay, IntegerType, LinkingScheme, Struct, Tuple,
};

pub type Result<T, E = Eip712Error> = core::result::Result<T, E>;

#[derive(Debug, Error, Clone)]
pub enum Eip712Error {
    #[error("Core error: {0}")]
    Core(#[from] core::fmt::Error),
    #[error("EIP712 ABI error: {0}")]
    Eip712Abi(#[from] alloy_dyn_abi::Error),
    #[error("The byte sequence could not be formatted for display: {0}")]
    InvalidBytes(#[from] byte_display::ByteFormatError),
    #[error("The input is not a valid utf-8 string: {0}")]
    InvalidString(#[from] core::str::Utf8Error),
    #[error("Invalid discriminant `{discriminant}` for {type_name}")]
    InvalidDiscriminant { type_name: String, discriminant: u8 },
    #[error(transparent)]
    UnresolvedType(#[from] ResolutionError),
    #[error("A discriminant is required for items of type `{type_name}` but the input ended without providing one.")]
    MissingDiscriminant { type_name: String },
    #[error("The input claimed to provide an integer {claimed_size} bytes wide, but only provided {bytes_available} additional bytes of input.")]
    MissingIntegerInput {
        claimed_size: u8,
        bytes_available: u8,
    },
    #[error("The input claimed to provide a byte array {claimed_size} bytes wide, but only provided {bytes_available} additional bytes of input.")]
    MissingBytesInput {
        claimed_size: usize,
        bytes_available: usize,
    },
    #[error("The input should have contained a vector but did not provide one.")]
    MissingVecLength,
    #[error("The input should have contained a string but did not provide one.")]
    MissingStringLength,
    #[error("Map field keys must map to a string to allow for valid EIP712 encoding. Field {0} contained an invalid key.")]
    InvalidMapKey(String),
    #[error("The provided input had leftover bytes that weren't displayed.")]
    UnusedInput,
}

/// The same Rust type in the schema can generate several solidity types. The simplest example of
/// this is an enum type with values from several variants (each one would be its own solidity
/// struct), but also e.g. optional values which are None are ommitted from the solidity struct so
/// even rust `struct`s can generate different solidity types.
/// We need to generate a unique type name if the field types are different, but without
/// duplicating types which are genuinely identical. We do this by appending a numbered suffix if
/// the Rust type name was already used for a different solidity type.
#[derive(Default)]
struct TypeVariants {
    /// Maps from solidity field definitions to the unique generated name for this variant
    variants: HashMap<Vec<PropertyDef>, String>,
    /// The next suffix number to use (0 means no suffix, 1 means "_1", etc.)
    next_suffix: usize,
}

pub struct Output<'t> {
    types: &'t mut Eip712Types,
    /// For each base (Rust) type name, tracks all its solidity variants that have already been
    /// generated (see `TypeVariants`).
    visited_types: HashMap<String, TypeVariants>,
}

impl<'t> Output<'t> {
    pub fn new(types: &'t mut Eip712Types) -> Self {
        Self {
            types,
            visited_types: Default::default(),
        }
    }

    /// Get or create a unique type name for the given field definitions.
    /// If this exact field definition already exists for this base type, returns the existing name.
    /// Otherwise, creates a new unique name with appropriate suffix.
    pub fn insert_types_and_get_or_create_name(
        &mut self,
        base_name: &str,
        fields: Vec<PropertyDef>,
    ) -> String {
        let type_info = self.visited_types.entry(base_name.to_string()).or_default();

        // Check if we've seen this exact field configuration for this base type before
        if let Some(existing_name) = type_info.variants.get(&fields) {
            return existing_name.clone();
        }

        // Generate a unique name for this variant
        let unique_name = if type_info.next_suffix == 0 {
            base_name.to_string()
        } else {
            format!("{}{}", base_name, type_info.next_suffix)
        };
        type_info.next_suffix += 1;
        type_info
            .variants
            .insert(fields.clone(), unique_name.clone());

        // Add to EIP712 types
        self.types.insert(unique_name.clone(), fields);

        unique_name
    }
}

pub struct Input<'a> {
    buf: &'a mut &'a [u8],
}

impl<'a> Input<'a> {
    pub fn new(buf: &'a mut &'a [u8]) -> Self {
        Self { buf }
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn check_remaining_bytes(&self, len: usize) -> Result<(), Eip712Error> {
        if self.buf.len() < len {
            return Err(Eip712Error::MissingBytesInput {
                claimed_size: len,
                bytes_available: self.buf.len(),
            });
        }
        Ok(())
    }

    /// Splits the first `len` bytes from the input, returning them as a slice and updating the input buffer.
    /// Returns an error if there are not enough bytes remaining to fulfill the request.
    pub fn advance(&mut self, len: usize) -> Result<&[u8], Eip712Error> {
        self.check_remaining_bytes(len)?;
        let (leading, rest) = self.buf.split_at(len);
        *self.buf = rest;
        Ok(leading)
    }
}

pub struct Eip712Visitor<'a, 't> {
    input: Input<'a>,
    output: Output<'t>,
}

impl<'a, 't> Eip712Visitor<'a, 't> {
    pub fn new(input: &'a mut &'a [u8], out_types: &'t mut Eip712Types) -> Self {
        Self {
            input: Input::new(input),
            output: Output::new(out_types),
        }
    }
}

impl Eip712Visitor<'_, '_> {
    pub fn has_displayed_whole_input(&self) -> bool {
        self.input.is_empty()
    }

    pub fn read_usize_borsh(&mut self) -> Result<usize, Eip712Error> {
        if self.input.len() < 4 {
            return Err(Eip712Error::MissingIntegerInput {
                claimed_size: 4,
                bytes_available: self.input.len() as u8,
            });
        }
        let len = u32::from_le_bytes(
            self.input
                .advance(4)?
                .try_into()
                .expect("Converting [u8;4] to u32 is infallible"),
        ) as usize;
        Ok(len)
    }

    fn display_byte_sequence(
        &mut self,
        len: usize,
        display: ByteDisplay,
        _context: Context,
    ) -> Result<Option<InnerReturnType>, Eip712Error> {
        self.input.check_remaining_bytes(len)?;
        let mut str = String::new();
        display.format(self.input.advance(len)?, &mut str)?;
        Ok(Some(InnerReturnType {
            json_value: serde_json::Value::String(str),
            unique_type_name: "string".to_string(),
        }))
    }

    /// Helper function for processing arrays and vectors, which share the same logic
    /// except for how they determine their length.
    fn process_sequence<L: LinkingScheme>(
        &mut self,
        len: usize,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Result<Option<InnerReturnType>, Eip712Error> {
        let inner = schema.resolve_or_err(value)?;
        let base_name = &context.parent_name;

        let mut json_values = Map::new();
        let mut inner_types = Vec::new();

        for i in 0..len {
            let Some(eip712_value) = inner.visit(
                schema,
                self,
                Context {
                    is_virtual: IsVirtual::No,
                    parent_name: format!("{base_name}_{i}"),
                },
            )?
            else {
                continue;
            };
            json_values.insert(i.to_string(), eip712_value.json_value);
            let property_def = PropertyDef::new(eip712_value.unique_type_name, i.to_string())?;
            inner_types.push(property_def);
        }

        let eip712_name = self
            .output
            .insert_types_and_get_or_create_name(base_name, inner_types);
        let json_value = serde_json::Value::Object(json_values);
        Ok(Some(InnerReturnType {
            json_value,
            unique_type_name: eip712_name,
        }))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    is_virtual: IsVirtual,
    /// For virtual structs or tuples, the parent name (which will be the enum variant name)
    /// replaces the type name. For non-virtual tuples, the parent name will be the field name and
    /// is the only type name we have.
    parent_name: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum IsVirtual {
    Yes,
    #[default]
    No,
}

// TODO: this would be nicer for devex if it were a function. The `<$t>::from_le_bytes` is what
// makes it non-trivial to convert though
macro_rules! display_int {
    ($t:ident, $input:expr, $disp:expr, $solint: expr) => {{
        let size = IntegerType::$t.size();
        if $input.len() < size {
            return Err(Eip712Error::MissingIntegerInput {
                claimed_size: size as u8,
                bytes_available: $input.len() as u8,
            });
        }
        let buf = $input.advance(size)?;
        let value = <$t>::from_le_bytes(buf.try_into().unwrap()).to_string();
        match $disp {
            IntegerDisplay::Hex => Ok(Some(InnerReturnType {
                json_value: serde_json::Value::String(hex::encode(value)),
                unique_type_name: "string".to_string(),
            })),
            // EIP-712 doesn't have support for fixed point decimals.
            IntegerDisplay::Decimal | IntegerDisplay::FixedPoint(_) => Ok(Some(InnerReturnType {
                json_value: serde_json::Value::String(value),
                unique_type_name: $solint.to_string(),
            })),
        }
    }};
}

pub struct InnerReturnType {
    pub json_value: serde_json::Value,
    pub unique_type_name: String,
}

impl<L: LinkingScheme, M> TypeVisitor<L, M> for Eip712Visitor<'_, '_> {
    type Arg = Context;
    type ReturnType = Result<Option<InnerReturnType>, Eip712Error>;
    fn visit_enum(
        &mut self,
        e: &Enum<L>,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        if self.input.is_empty() && !e.variants.is_empty() {
            return Err(Eip712Error::MissingDiscriminant {
                type_name: e.type_name.clone(),
            });
        }

        let discriminant = self.input.advance(1)?[0];
        let mut variants_by_discriminant =
            e.variants.iter().filter(|v| v.discriminant == discriminant);
        let variant = variants_by_discriminant
            .next()
            .ok_or(Eip712Error::InvalidDiscriminant {
                type_name: e.type_name.clone(),
                discriminant,
            })?;
        assert!(variants_by_discriminant.next().is_none(), "Found two enum variants with the same discriminant - the schema is malformed, cannot proceed!");

        // The enum type name comes from context (for nested) or from the enum itself
        let enum_type_name = if context.is_virtual == IsVirtual::Yes {
            &context.parent_name
        } else {
            &e.type_name
        };

        // Create the single-field struct for the enum
        let mut field_values = Map::new();
        let mut field_types = Vec::new();

        let (field_value, field_type_name) = if let Some(maybe_resolved) = &variant.value {
            // Visit the variant's content with virtual context
            let inner = schema.resolve_or_err(maybe_resolved)?;
            let variant_context = Context {
                is_virtual: IsVirtual::Yes,
                parent_name: variant.name.clone(),
            };

            if let Some(result) = inner.visit(schema, self, variant_context)? {
                (result.json_value, result.unique_type_name)
            } else {
                // Skip field if the inner type returns None
                return Ok(None);
            }
        } else {
            // Empty variant - use bool type with value true
            (serde_json::Value::Bool(true), "bool".to_string())
        };

        // Add the variant as a field in the enum struct
        field_values.insert(variant.name.clone(), field_value);
        let property_def = PropertyDef::new(field_type_name, variant.name.clone())?;
        field_types.push(property_def);

        // Register this enum variant configuration and get its unique name
        let eip712_name = self
            .output
            .insert_types_and_get_or_create_name(enum_type_name, field_types);
        let json_value = serde_json::Value::Object(field_values);

        Ok(Some(InnerReturnType {
            json_value,
            unique_type_name: eip712_name,
        }))
    }

    fn visit_struct(
        &mut self,
        s: &Struct<L>,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        // 1. Get the type names and JSON values for every child field
        // 2. Determine the type name, uniquely based on the subtypes
        // 3. Construct type array and insert into output types with the provided name
        // 4. Construct inner json: start an object and insert the child value JSONs
        let base_name = if context.is_virtual == IsVirtual::Yes {
            &context.parent_name
        } else {
            &s.type_name
        };

        let mut field_values = Map::new();
        let mut field_types = Vec::new();

        for field in &s.fields {
            let inner_ty = schema.resolve_or_err(&field.value)?;
            let Some(eip712_field) = inner_ty.visit(
                schema,
                self,
                Context {
                    is_virtual: IsVirtual::No,
                    parent_name: field.display_name.clone(),
                },
            )?
            else {
                continue;
            };
            if !field.silent && !inner_ty.is_skip() {
                field_values.insert(field.display_name.clone(), eip712_field.json_value);
                let property_def =
                    PropertyDef::new(eip712_field.unique_type_name, field.display_name.clone())?;
                field_types.push(property_def);
            }
        }
        let eip712_name = self
            .output
            .insert_types_and_get_or_create_name(base_name, field_types);
        let json_value = serde_json::Value::Object(field_values);
        Ok(Some(InnerReturnType {
            json_value,
            unique_type_name: eip712_name,
        }))
    }

    fn visit_tuple(
        &mut self,
        t: &Tuple<L>,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        // Trivial tuple (single field) - always transparent
        // The magic happens in visit_enum when it sees a virtual context
        if t.fields.len() == 1 {
            let field = &t.fields[0];
            let inner_ty = schema.resolve_or_err(&field.value)?;
            // Pass through the context - if we're virtual, the inner enum will use parent_name as its type
            return inner_ty.visit(schema, self, context);
        }

        // Non-trivial tuple - treat like a struct with numeric field names
        let base_name = &context.parent_name;

        let mut field_values = Map::new();
        let mut field_types = Vec::new();

        for (i, field) in t.fields.iter().enumerate() {
            let inner_ty = schema.resolve_or_err(&field.value)?;
            let field_name = i.to_string();
            let Some(eip712_field) = inner_ty.visit(
                schema,
                self,
                Context {
                    is_virtual: IsVirtual::No,
                    parent_name: format!("{base_name}_{field_name}"),
                },
            )?
            else {
                continue;
            };

            if !field.silent && !inner_ty.is_skip() {
                field_values.insert(field_name.clone(), eip712_field.json_value);
                let property_def = PropertyDef::new(eip712_field.unique_type_name, field_name)?;
                field_types.push(property_def);
            }
        }

        let eip712_name = self
            .output
            .insert_types_and_get_or_create_name(base_name, field_types);
        let json_value = serde_json::Value::Object(field_values);
        Ok(Some(InnerReturnType {
            json_value,
            unique_type_name: eip712_name,
        }))
    }

    fn visit_option(
        &mut self,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Self::Arg,
    ) -> Self::ReturnType {
        let discriminant = self.input.advance(1)?[0];

        match discriminant {
            0 => Ok(None),
            1 => schema.resolve_or_err(value)?.visit(schema, self, context),
            _ => Err(Eip712Error::InvalidDiscriminant {
                type_name: "Option".to_string(),
                discriminant,
            }),
        }
    }

    fn visit_primitive(
        &mut self,
        p: crate::schema::Primitive,
        _schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        match p {
            Primitive::Float32 => {
                let value = self.input.advance(4)?;
                let value = f32::from_le_bytes(value.try_into().unwrap()).to_string();
                let json_value = serde_json::Value::String(value);
                Ok(Some(InnerReturnType {
                    json_value,
                    unique_type_name: "string".to_string(),
                }))
            }
            Primitive::Float64 => {
                let value = self.input.advance(8)?;
                let value = f64::from_le_bytes(value.try_into().unwrap()).to_string();
                let json_value = serde_json::Value::String(value);
                Ok(Some(InnerReturnType {
                    json_value,
                    unique_type_name: "string".to_string(),
                }))
            }
            Primitive::Boolean => {
                let value = self.input.advance(1)?;
                let json_value = match value[0] {
                    0 => serde_json::Value::Bool(false),
                    1 => serde_json::Value::Bool(true),
                    _ => {
                        return Err(Eip712Error::InvalidDiscriminant {
                            type_name: "bool".to_string(),
                            discriminant: value[0],
                        });
                    }
                };
                Ok(Some(InnerReturnType {
                    json_value,
                    unique_type_name: "bool".to_string(),
                }))
            }
            Primitive::Integer(int, display) => match int {
                IntegerType::i8 => display_int!(i8, self.input, display, "int8"),
                IntegerType::i16 => display_int!(i16, self.input, display, "int16"),
                IntegerType::i32 => display_int!(i32, self.input, display, "int32"),
                IntegerType::i64 => display_int!(i64, self.input, display, "int64"),
                IntegerType::i128 => display_int!(i128, self.input, display, "int128"),
                IntegerType::u8 => display_int!(u8, self.input, display, "uint8"),
                IntegerType::u16 => display_int!(u16, self.input, display, "uint16"),
                IntegerType::u32 => display_int!(u32, self.input, display, "uint32"),
                IntegerType::u64 => display_int!(u64, self.input, display, "uint64"),
                IntegerType::u128 => display_int!(u128, self.input, display, "uint128"),
            },
            Primitive::ByteArray { len, display } => {
                self.display_byte_sequence(len, display, context)
            }
            Primitive::ByteVec { display } => {
                let len = self
                    .read_usize_borsh()
                    .or(Err(Eip712Error::MissingVecLength))?;
                self.display_byte_sequence(len, display, context)
            }
            Primitive::String => {
                let len = self
                    .read_usize_borsh()
                    .or(Err(Eip712Error::MissingStringLength))?;
                let content = self.input.advance(len)?;
                let content = std::str::from_utf8(content)?.to_string();
                Ok(Some(InnerReturnType {
                    json_value: serde_json::Value::String(content),
                    unique_type_name: "string".to_string(),
                }))
            }
            Primitive::Skip { len } => {
                self.input.advance(len)?;
                Ok(None)
            }
        }
    }

    fn visit_array(
        &mut self,
        len: &usize,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        self.process_sequence(*len, value, schema, context)
    }

    fn visit_vec(
        &mut self,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        let len = self.read_usize_borsh()?;
        self.process_sequence(len, value, schema, context)
    }

    fn visit_map(
        &mut self,
        key: &L::TypeLink,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        let len = self.read_usize_borsh()?;
        let key = schema.resolve_or_err(key)?;
        let value = schema.resolve_or_err(value)?;

        let base_name = &context.parent_name;

        let mut json_values = Map::new();
        let mut inner_types = Vec::new();

        for i in 0..len {
            let Some(eip712_key) = key.visit(
                schema,
                self,
                Context {
                    is_virtual: IsVirtual::No,
                    parent_name: format!("{base_name}_{i}"),
                },
            )?
            else {
                return Err(Eip712Error::InvalidMapKey(base_name.clone()));
            };
            let key_name = match eip712_key.json_value {
                serde_json::Value::String(s) => s,
                _ => return Err(Eip712Error::InvalidMapKey(base_name.clone())),
            };

            let Some(eip712_value) = value.visit(schema, self, context.clone())? else {
                continue;
            };

            json_values.insert(key_name.clone(), eip712_value.json_value);
            let property_def = PropertyDef::new(eip712_value.unique_type_name, key_name)?;
            inner_types.push(property_def);
        }

        let eip712_name = self
            .output
            .insert_types_and_get_or_create_name(base_name, inner_types);
        let json_value = serde_json::Value::Object(json_values);
        Ok(Some(InnerReturnType {
            json_value,
            unique_type_name: eip712_name,
        }))
    }
}
