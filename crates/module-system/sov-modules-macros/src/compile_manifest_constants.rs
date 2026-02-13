use proc_macro2::{Ident, Span, TokenStream};
use quote::format_ident;
use syn::punctuated::Punctuated;

use crate::manifest::Manifest;

#[derive(Clone)]
pub struct ConfigValueInput {
    pub constant_name: syn::LitStr,
    pub custom_error_message: Option<syn::LitStr>,
    pub span: Span,
}

impl syn::parse::Parse for ConfigValueInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let span = input.span();
        let constant_name = input.parse()?;

        let custom_error_message = if input.peek(syn::token::Comma) {
            input.parse::<syn::token::Comma>()?;
            Some(input.parse()?)
        } else {
            None
        };

        Ok(Self {
            constant_name,
            custom_error_message,
            span,
        })
    }
}

pub fn make_const_value(input: &ConfigValueInput) -> syn::Result<TokenStream> {
    // By isolating the core logic in a separate function, we can keep on using
    // ? and only custom error message logic at the end.
    make_const_value_inner(input).map_err(|error| {
        if let Some(custom_error_message) = &input.custom_error_message {
            let mut e = syn::Error::new(input.span, custom_error_message.value());
            e.combine(error);
            e
        } else {
            error
        }
    })
}

pub fn make_const_value_inner(input: &ConfigValueInput) -> syn::Result<TokenStream> {
    // Parse the manifest...
    let field_ident = Ident::new(&input.constant_name.value(), input.constant_name.span());
    let manifest = Manifest::read_constants(&field_ident)?;

    // ... and extract the TOML value.
    let toml_value = manifest.get(&field_ident)?;
    // Finally, compile it into a Rust expression.
    let rust_expr = compile_toml_value_to_rust(toml_value, input)?;

    Ok(quote::quote!(#rust_expr))
}

#[derive(serde::Deserialize)]
pub struct TomlConstValue {
    #[serde(rename = "const")]
    const_value: toml::Value,
}

#[derive(serde::Deserialize)]
pub struct TomlBech32Value {
    bech32: String,
    r#type: String,
}

#[derive(serde::Deserialize)]
pub struct TomlHexValue {
    hex: String,
}

#[derive(serde::Deserialize)]
pub struct TomlByteStringValue {
    byte_string: String,
}

/// A chain hash override for a range of block heights.
/// Used in constants.toml as: `{ start_height = <height>, end_height = <height>, chain_hash = "0x...", grace_period = <blocks> }`
#[derive(serde::Deserialize)]
pub struct TomlChainHashOverride {
    /// The start height (inclusive).
    pub start_height: u64,
    /// The end height (exclusive).
    pub end_height: u64,
    /// The chain hash as a hex string (with or without "0x" prefix).
    pub chain_hash: String,
    /// Number of blocks after end_height during which this hash is still accepted.
    /// During the grace period, both this hash and the next override's hash are valid.
    #[serde(default)]
    pub grace_period: u64,
}

pub enum AllowedTomlValue {
    Bool(bool),
    Integer(i64),
    String(String),
    Array(Vec<AllowedTomlValue>),
    Bech32(TomlBech32Value),
    Hex(TomlHexValue),
    ByteString(TomlByteStringValue),
    ChainHashOverride(TomlChainHashOverride),
}

pub struct ParsedConstant {
    pub value: AllowedTomlValue,
    // Whether or not the constant should be `const`-ifiable (i.e. non-overridable).
    pub make_const: bool,
}

fn parse_constant_inner(value: &toml::Value, span: Span) -> syn::Result<AllowedTomlValue> {
    use toml::Value;

    let error = |toml_type: &str| {
        syn::Error::new(span, format!("failed to convert TOML value into Rust expression; its TOML value type ({toml_type}) is not supported: `{value:?}`"))
    };

    match value {
        Value::Table(_) => {
            if let Ok(bech32_table) = value.clone().try_into::<TomlBech32Value>() {
                Ok(AllowedTomlValue::Bech32(bech32_table))
            } else if let Ok(hex_table) = value.clone().try_into::<TomlHexValue>() {
                Ok(AllowedTomlValue::Hex(hex_table))
            } else if let Ok(byte_string_table) = value.clone().try_into::<TomlByteStringValue>() {
                Ok(AllowedTomlValue::ByteString(byte_string_table))
            } else if let Ok(chain_hash_override) =
                value.clone().try_into::<TomlChainHashOverride>()
            {
                Ok(AllowedTomlValue::ChainHashOverride(chain_hash_override))
            } else {
                Err(error("table"))
            }
        }
        // Floats can result in diverging behavior between zkVMs and native
        // execution: <https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/909>.
        Value::Float(_) => Err(error("float")),
        // We wouldn't be quite sure what type to represent TOML datetimes as...
        // so, for lack of a good option, and because of their reduced utility
        // for most applications, we don't support them.
        Value::Datetime(_) => Err(error("datetime")),
        Value::Boolean(b) => Ok(AllowedTomlValue::Bool(*b)),
        Value::Integer(num) => Ok(AllowedTomlValue::Integer(*num)),
        Value::String(s) => Ok(AllowedTomlValue::String(s.clone())),
        Value::Array(arr) => {
            let values: Vec<AllowedTomlValue> = arr
                .iter()
                .map(|v| parse_constant_inner(v, span))
                .collect::<syn::Result<_>>()?;
            Ok(AllowedTomlValue::Array(values))
        }
    }
}

fn parse_constant(value: &toml::Value, span: Span) -> syn::Result<ParsedConstant> {
    // Arrays and non-table types should always be handled by parse_constant_inner directly.
    // We only try struct deserialization for TOML tables, because the toml serde deserializer
    // can spuriously unwrap single-element arrays into structs with one field.
    if !value.is_table() {
        return Ok(ParsedConstant {
            value: parse_constant_inner(value, span)?,
            make_const: false,
        });
    }

    // Is it a `{ const = ... }` value?
    if let Ok(with_custom_override) = value.clone().try_into::<TomlConstValue>() {
        Ok(ParsedConstant {
            value: parse_constant_inner(&with_custom_override.const_value, span)?,
            make_const: true,
        })
    } else if let Ok(hex_value) = value.clone().try_into::<TomlHexValue>() {
        // Hex values are always const (no runtime overrides)
        Ok(ParsedConstant {
            value: AllowedTomlValue::Hex(hex_value),
            make_const: true,
        })
    } else if let Ok(byte_string_value) = value.clone().try_into::<TomlByteStringValue>() {
        // Byte string values are always const (no runtime overrides)
        Ok(ParsedConstant {
            value: AllowedTomlValue::ByteString(byte_string_value),
            make_const: true,
        })
    } else {
        Ok(ParsedConstant {
            value: parse_constant_inner(value, span)?,
            make_const: false,
        })
    }
}

/// Validates that an array of ChainHashOverride values is contiguous and starts at zero.
/// Returns Ok(()) if the array contains no ChainHashOverride values (i.e., it's a different
/// type of array). Errors if the array contains a mix of ChainHashOverride and other types.
fn validate_chain_hash_override_array(
    constant_name: &syn::LitStr,
    arr: &[AllowedTomlValue],
) -> syn::Result<()> {
    // Collect all ChainHashOverride values from the array
    let overrides: Vec<&TomlChainHashOverride> = arr
        .iter()
        .filter_map(|v| match v {
            AllowedTomlValue::ChainHashOverride(o) => Some(o),
            _ => None,
        })
        .collect();

    // If no ChainHashOverride values, this is a different type of array - skip validation
    if overrides.is_empty() {
        return Ok(());
    }

    // If some but not all elements are ChainHashOverride, that's an error
    if overrides.len() != arr.len() {
        return Err(syn::Error::new(
            constant_name.span(),
            "Chain hash override array contains mixed types; all elements must be chain hash overrides",
        ));
    }

    // Validate: first override must start at 0
    if overrides[0].start_height != 0 {
        return Err(syn::Error::new(
            constant_name.span(),
            format!(
                "Chain hash overrides must start at height 0, but first override starts at {}",
                overrides[0].start_height
            ),
        ));
    }

    // Validate: each subsequent override must start where the previous one ended
    for window in overrides.windows(2) {
        if window[1].start_height != window[0].end_height {
            return Err(syn::Error::new(
                constant_name.span(),
                format!(
                    "Chain hash overrides must be contiguous: override ending at {} is followed by override starting at {}",
                    window[0].end_height,
                    window[1].start_height
                ),
            ));
        }
    }

    Ok(())
}

fn allowed_toml_value_to_const_expr(
    constant_name: &syn::LitStr,
    value: &AllowedTomlValue,
) -> syn::Result<syn::Expr> {
    Ok(match value {
        AllowedTomlValue::String(s) => syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Str(syn::LitStr::new(s, Span::call_site())),
        }),
        AllowedTomlValue::Bool(b) => syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Bool(syn::LitBool::new(*b, Span::call_site())),
        }),
        AllowedTomlValue::Integer(i) => syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Int(syn::LitInt::new(&i.to_string(), Span::call_site())),
        }),
        AllowedTomlValue::Array(arr) => {
            // Check if this is an array of ChainHashOverride - if so, validate at compile time
            validate_chain_hash_override_array(constant_name, arr)?;

            let values = arr
                .iter()
                .map(|v| allowed_toml_value_to_const_expr(constant_name, v))
                .collect::<syn::Result<Vec<_>>>()?;

            // HACK: Apply special casing for the chain hash overrides array when given an empty array,
            // But only if we're actually handling the CHAIN_HASH_OVERRIDES constant.
            let might_be_chain_hash_array = is_chain_hash_override_array(arr)
                || (values.is_empty() && constant_name.value() == "CHAIN_HASH_OVERRIDES");
            // We return slices instead of raw arrays for the chain hash overrides. This is because the length of the array is unknown.
            if might_be_chain_hash_array {
                syn::parse_quote!([#(#values),*].as_slice())
            } else {
                syn::Expr::Array(syn::ExprArray {
                    attrs: Vec::new(),
                    bracket_token: syn::token::Bracket::default(),
                    elems: Punctuated::from_iter(values),
                })
            }
        }
        AllowedTomlValue::Bech32(bech32) => {
            let bech32_type = format_ident!("{}", bech32.r#type);
            toml_bech32_value_to_rust(constant_name, &bech32.bech32, &bech32_type)?
        }
        AllowedTomlValue::Hex(hex) => toml_hex_value_to_rust(constant_name, &hex.hex)?,
        AllowedTomlValue::ByteString(byte_string) => {
            toml_byte_string_value_to_rust(constant_name, &byte_string.byte_string)?
        }
        AllowedTomlValue::ChainHashOverride(override_) => {
            toml_chain_hash_override_to_rust(constant_name, override_)?
        }
    })
}

/// Returns true if the array contains ChainHashOverride elements.
fn is_chain_hash_override_array(arr: &[AllowedTomlValue]) -> bool {
    arr.first()
        .map(|v| matches!(v, AllowedTomlValue::ChainHashOverride(_)))
        .unwrap_or(false)
}

/// Generates the override logic expression for ChainHashOverride arrays.
fn chain_hash_override_array_override_logic() -> TokenStream {
    // ChainHashOverride arrays need special handling: deserialize with string chain_hash,
    // then convert hex strings to [u8; 32]
    quote::quote!({
        use sov_modules_api::prelude::{serde, toml};

        // Intermediate struct for deserialization with string chain_hash
        #[derive(serde::Deserialize)]
        #[serde(default)]
        struct RawChainHashOverride {
            start_height: u64,
            end_height: u64,
            chain_hash: String,
            grace_period: u64,
        }

        impl Default for RawChainHashOverride {
            fn default() -> Self {
                Self {
                    start_height: 0,
                    end_height: 0,
                    chain_hash: String::new(),
                    grace_period: 0,
                }
            }
        }

        let deserializer = toml::de::ValueDeserializer::new(&env_value);
        let raw_overrides: Vec<RawChainHashOverride> =
            serde::Deserialize::deserialize(deserializer).unwrap();

        // Convert hex strings to [u8; 32]
        let overrides = raw_overrides
            .into_iter()
            .map(|raw| {
                let hex_str = raw.chain_hash.strip_prefix("0x").unwrap_or(&raw.chain_hash);
                let hash_bytes: [u8; 32] = hex::decode(hex_str)
                    .expect("Invalid hex string in chain hash override")
                    .try_into()
                    .expect("Chain hash must be exactly 32 bytes");
                sov_modules_api::ChainHashOverride {
                    start_height: raw.start_height,
                    end_height: raw.end_height,
                    chain_hash: hash_bytes,
                    grace_period: raw.grace_period,
                }
            })
            .collect::<Vec<_>>()
            .leak(); // Because we need the type of the result to be slice (to match the type returned by the function when overrides
                     //are disabled), we have to leak the vector. Overrides are only active when debug assertions are enabled, so this is no big deal

        &overrides[..]
    })
}

fn allowed_toml_value_to_expr_with_override_logic(
    constant_name: &str,
    value: &AllowedTomlValue,
) -> TokenStream {
    match value {
        AllowedTomlValue::Bool(_) | AllowedTomlValue::Integer(_) | AllowedTomlValue::Bech32(_) => {
            quote::quote!({ str::parse(&env_value).unwrap() })
        }
        AllowedTomlValue::String(_) => quote::quote!({
            // We need a value of the same type as string literal i.e. `&'static str`.
            // The only way to do that starting from a `String` is to leak it.
            //
            // TODO(@neysofu, improvement): cache it somewhere so it's only
            // leaked once? See <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2510>.
            &*env_value.leak()
        }),
        AllowedTomlValue::Array(arr)
        // HACK: Apply special casing for the chain hash overrides array when given an empty array, 
        // But only if we're actually handling the CHAIN_HASH_OVERRIDES constant.
        // Special casing is required because the length of the array is unknown.
            if is_chain_hash_override_array(arr) || (arr.is_empty() && constant_name== "CHAIN_HASH_OVERRIDES") =>
        {
            chain_hash_override_array_override_logic()
        }
        AllowedTomlValue::Array(_)
        | AllowedTomlValue::Hex(_)
        | AllowedTomlValue::ByteString(_)
        | AllowedTomlValue::ChainHashOverride(_) => {
            quote::quote!({
                use sov_modules_api::prelude::{serde, toml};

                let deserializer = toml::de::ValueDeserializer::new(&env_value);
                let owned: Vec<_> = serde::Deserialize::deserialize(deserializer).unwrap();
                owned.try_into().unwrap()
            })
        }
    }
}

/// Converts a TOML value into a Rust expression of the most appropriate type.
///
/// Nulls and objects are not supported because they don't map naturally to any
/// [`std`] Rust type.
pub fn compile_toml_value_to_rust(
    value: &toml::Value,
    input: &ConfigValueInput,
) -> syn::Result<proc_macro2::TokenStream> {
    const CONST_VALUE_OVERRIDE_ENV_VAR_PREFIX: &str = "SOV_TEST_CONST_OVERRIDE_";

    let env_var_name = format!(
        "{}{}",
        CONST_VALUE_OVERRIDE_ENV_VAR_PREFIX,
        input.constant_name.value()
    );

    let parsed_constant = parse_constant(value, input.span)?;
    let const_expr =
        allowed_toml_value_to_const_expr(&input.constant_name, &parsed_constant.value)?;

    Ok(if parsed_constant.make_const {
        quote::quote!(#const_expr)
    } else {
        let non_const_expr = allowed_toml_value_to_expr_with_override_logic(
            &input.constant_name.value(),
            &parsed_constant.value,
        );

        quote::quote!({
            #[cfg(debug_assertions)]
            {
                fn ERROR___you_must_use_const_equals_in_toml_file_to_use_this_proc_macro_in_const_expressions() {}
                ERROR___you_must_use_const_equals_in_toml_file_to_use_this_proc_macro_in_const_expressions();

                if let Ok(env_value) = ::std::env::var(#env_var_name) {
                    #non_const_expr
                } else {
                    #const_expr
                }
            }
            #[cfg(not(debug_assertions))]
            {
                #const_expr
            }
        })
    })
}

pub fn toml_bech32_value_to_rust(
    constant_name: &syn::LitStr,
    bech32_str: &str,
    bech32_wrapper_type: &syn::Ident,
) -> syn::Result<syn::Expr> {
    use bech32::primitives::decode::CheckedHrpstring;
    use bech32::Bech32m;

    // Parse the string literal as a bech32 string.
    let type_name = quote::quote! { #bech32_wrapper_type };
    let hrp_string = CheckedHrpstring::new::<Bech32m>(bech32_str).map_err(|err| {
        syn::Error::new(
            constant_name.span(),
            format!(
                "The constant named `{}` is not a valid bech32 string; decoding failed: {}",
                constant_name.value(),
                err
            ),
        )
    })?;
    let bech32_bytes = hrp_string.byte_iter();
    let found_prefix = hrp_string.hrp().to_string();

    let const_expr_tokens = {
        // Generate the code which will check that the HRP (Human-Readable Part) is correct.
        let bad_prefix_error = format!(
            "The constant named `{}` with value `{}` is a valid bech32 string but its prefix `{}` does not match the expected prefix for the type `{}`.",
            constant_name.value(),
            bech32_str,
            found_prefix,
            type_name
        );

        // We can't loop in a `const fn`, so we generate an `assert!` for each byte.
        let mut assertions = vec![];
        for (idx, byte) in found_prefix.as_bytes().iter().enumerate() {
            assertions.push(quote::quote! {
                assert!(#byte == #found_prefix.as_bytes()[#idx], #bad_prefix_error);
            });
        }

        quote::quote!({
            if #found_prefix.len() != #type_name::bech32_prefix().len() {
                panic!(#bad_prefix_error);
            }
            #(#assertions)*

            #bech32_wrapper_type::from_const_slice( [#(#bech32_bytes,)*] )
        })
    };

    syn::parse2(const_expr_tokens)
}

pub fn toml_hex_value_to_rust(
    constant_name: &syn::LitStr,
    hex_str: &str,
) -> syn::Result<syn::Expr> {
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(hex_str)
        .map_err(|e| syn::Error::new(constant_name.span(), format!("Invalid hex string: {e}")))?;

    bytes_to_array_expr(&bytes)
}

pub fn toml_byte_string_value_to_rust(
    _constant_name: &syn::LitStr,
    string: &str,
) -> syn::Result<syn::Expr> {
    bytes_to_array_expr(string.as_bytes())
}

fn bytes_to_array_expr(bytes: &[u8]) -> syn::Result<syn::Expr> {
    // Generate [1u8, 2u8, 3u8, ...] with u8 suffix for type inference
    let elems = bytes.iter().map(|b| {
        let lit = syn::LitInt::new(&format!("{b}u8"), Span::call_site());
        syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Int(lit),
        })
    });

    Ok(syn::Expr::Array(syn::ExprArray {
        attrs: Vec::new(),
        bracket_token: syn::token::Bracket::default(),
        elems: Punctuated::from_iter(elems),
    }))
}

pub fn toml_chain_hash_override_to_rust(
    constant_name: &syn::LitStr,
    hash_override: &TomlChainHashOverride,
) -> syn::Result<syn::Expr> {
    let start_height = hash_override.start_height;
    let end_height = hash_override.end_height;
    let grace_period = hash_override.grace_period;

    let hex_str = hash_override
        .chain_hash
        .strip_prefix("0x")
        .unwrap_or(&hash_override.chain_hash);
    let hash_bytes = hex::decode(hex_str).map_err(|e| {
        syn::Error::new(
            constant_name.span(),
            format!("Invalid hex string in chain hash override: {e}"),
        )
    })?;

    if hash_bytes.len() != 32 {
        return Err(syn::Error::new(
            constant_name.span(),
            format!(
                "Chain hash override hash must be exactly 32 bytes, got {}",
                hash_bytes.len()
            ),
        ));
    }

    let hash_bytes_tokens = hash_bytes.iter().map(|b| {
        let lit = syn::LitInt::new(&format!("{b}u8"), Span::call_site());
        syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Int(lit),
        })
    });

    let const_expr_tokens = quote::quote!({
        sov_modules_api::ChainHashOverride {
            start_height: #start_height,
            end_height: #end_height,
            chain_hash: [#(#hash_bytes_tokens),*],
            grace_period: #grace_period,
        }
    });

    syn::parse2(const_expr_tokens)
}
