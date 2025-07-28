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

pub enum AllowedTomlValue {
    Bool(bool),
    Integer(i64),
    String(String),
    Array(Vec<AllowedTomlValue>),
    Bech32(TomlBech32Value),
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
    // Is it a `{ const = ... }` value?
    if let Ok(with_custom_override) = value.clone().try_into::<TomlConstValue>() {
        Ok(ParsedConstant {
            value: parse_constant_inner(&with_custom_override.const_value, span)?,
            make_const: true,
        })
    } else {
        Ok(ParsedConstant {
            value: parse_constant_inner(value, span)?,
            make_const: false,
        })
    }
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
            let values = arr
                .iter()
                .map(|v| allowed_toml_value_to_const_expr(constant_name, v))
                .collect::<syn::Result<Vec<_>>>()?;
            syn::Expr::Array(syn::ExprArray {
                attrs: Vec::new(),
                bracket_token: syn::token::Bracket::default(),
                elems: Punctuated::from_iter(values),
            })
        }
        AllowedTomlValue::Bech32(bech32) => {
            let bech32_type = format_ident!("{}", bech32.r#type);
            toml_bech32_value_to_rust(constant_name, &bech32.bech32, &bech32_type)?
        }
    })
}

fn allowed_toml_value_to_expr_with_override_logic(value: &AllowedTomlValue) -> TokenStream {
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
        AllowedTomlValue::Array(_) => quote::quote!({
            use sov_modules_api::prelude::{serde, toml};

            let deserializer = toml::de::ValueDeserializer::new(&env_value);
            let owned: Vec<_> = serde::Deserialize::deserialize(deserializer).unwrap();
            owned.try_into().unwrap()
        }),
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
        let non_const_expr = allowed_toml_value_to_expr_with_override_logic(&parsed_constant.value);

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
