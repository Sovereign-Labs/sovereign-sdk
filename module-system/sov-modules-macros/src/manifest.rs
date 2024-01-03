use std::path::{Path, PathBuf};
use std::{env, fmt, fs, ops, process};

use proc_macro2::{Ident, TokenStream};
use quote::format_ident;
use serde_json::{Map, Value};
use syn::{PathArguments, Type, TypePath};

#[derive(Debug, Clone)]
pub struct Manifest<'a> {
    parent: &'a Ident,
    path: PathBuf,
    value: Value,
}

impl<'a> ops::Deref for Manifest<'a> {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a> Manifest<'a> {
    /// Parse a manifest file from a string.
    ///
    /// The provided path will be used to feedback error to the user, if any.
    ///
    /// The `parent` is used to report the errors to the correct span location.
    pub fn read_str<S>(manifest: S, path: PathBuf, parent: &'a Ident) -> Result<Self, syn::Error>
    where
        S: AsRef<str>,
    {
        let value = serde_json::from_str(manifest.as_ref())
            .map_err(|e| Self::err(&path, parent, format!("failed to parse manifest: {e}")))?;

        Ok(Self {
            parent,
            path,
            value,
        })
    }

    /// Reads a `constants.json` manifest file, retrieving it from the workspace root of the
    /// current working directory.
    ///
    /// If the environment variable `CONSTANTS_MANIFEST` is set, it will use that path as workspace
    /// directory.
    ///
    /// If the compilation is executed for a directory different than the current working dir
    /// (example: `cargo build --manifest-path /foo/bar/Cargo.toml`), you should override the
    /// constants manifest dir with the target directory:
    ///
    /// ```sh
    /// CONSTANTS_MANIFEST=/foo/bar cargo build --manifest-path /foo/bar/Cargo.toml
    /// ```
    ///
    /// The `parent` is used to report the errors to the correct span location.
    pub fn read_constants(parent: &'a Ident) -> Result<Self, syn::Error> {
        #[cfg(not(test))]
        let mut name = "constants.json";

        #[cfg(test)]
        let mut name = "constants.test.json";

        // workaround to https://github.com/dtolnay/trybuild/issues/231
        // despite trybuild being a crate to build tests, it won't set the `test` flag. It isn't
        // setting the `trybuild` flag properly either.
        if env::var_os("CONSTANTS_MANIFEST_TRYBUILD").is_some() {
            name = "constants.test.json";
        }

        let constants_dir = env::var_os("CONSTANTS_MANIFEST")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(env::current_dir)
            .map_err(|e| {
                Self::err(
                    env!("CARGO_MANIFEST_DIR"),
                    parent,
                    format!("failed to compute the `{name}` base path: {e}"),
                )
            })?;

        // we remove the __CARGO_FIX_PLZ due to incompatibility with `cargo metadata`
        // https://github.com/rust-lang/cargo/issues/9706
        let output = process::Command::new(env!("CARGO"))
            .args(["metadata", "--format-version=1", "--no-deps"])
            .current_dir(&constants_dir)
            .env_remove("__CARGO_FIX_PLZ")
            .output()
            .map_err(|e| {
                Self::err(
                    &constants_dir,
                    parent,
                    format!("failed to compute the `{name}` path: {e}"),
                )
            })?;

        let metadata: Value = serde_json::from_slice::<Value>(&output.stdout).map_err(|e| {
            Self::err(
                &constants_dir,
                parent,
                format!("Failed to parse `workspace_root` as json: {}", e),
            )
        })?;
        let ws_root = metadata.get("workspace_root").ok_or_else(|| {
            Self::err(
                &constants_dir,
                parent,
                "Failed to read `workspace_root` from cargo metadata",
            )
        })?;
        let ws = ws_root
            .as_str()
            .ok_or_else(|| {
                Self::err(
                    &constants_dir,
                    parent,
                    "The `workspace_root` from cargo metadata is not a valid string",
                )
            })
            .map(PathBuf::from)?;

        if !ws.is_dir() {
            return Err(Self::err(
                &ws,
                parent,
                format!("the computed `{name}` path is not a directory"),
            ));
        }

        // checks if is pointing to a cargo project
        if !ws.join("Cargo.toml").is_file() {
            return Err(Self::err(
                &ws,
                parent,
                format!(
                    "the computed `{name}` path is not a valid workspace: Cargo.toml not found"
                ),
            ));
        }

        let constants_path = ws.join(name);
        let constants = fs::read_to_string(&constants_path).map_err(|e| {
            Self::err(
                &constants_path,
                parent,
                format!("failed to read `{}`: {}", constants_path.display(), e),
            )
        })?;

        Self::read_str(constants, constants_path, parent)
    }

    /// Gets the requested object from the manifest by key
    fn get_object(&self, field: &Ident, key: &str) -> Result<&Map<String, Value>, syn::Error> {
        self.value
            .as_object()
            .ok_or_else(|| Self::err(&self.path, field, "manifest is not an object"))?
            .get(key)
            .ok_or_else(|| {
                Self::err(
                    &self.path,
                    field,
                    format!("manifest does not contain a `{key}` attribute"),
                )
            })?
            .as_object()
            .ok_or_else(|| {
                Self::err(
                    &self.path,
                    field,
                    format!("`{key}` attribute of `{field}` is not an object"),
                )
            })
    }

    /// Parses a gas config constant from the manifest file. Returns a `TokenStream` with the
    /// following structure:
    ///
    /// ```rust,ignore
    /// const GAS_CONFIG: Self::GasConfig = Self::GasConfig {
    ///     foo: [1u64, 2u64, 3u64, ],
    ///     bar: [4u64, 5u64, 6u64, ],
    /// };
    /// ```
    ///
    /// Where `foo` and `bar` are fields of the json constants file under the located `gas` field.
    ///
    /// The `gas` field resolution will first attempt to query `gas.parent`, and then fallback to
    /// `gas`. They must be objects with arrays of integers as fields.
    pub fn parse_gas_config(&self, ty: &Type, field: &Ident) -> Result<TokenStream, syn::Error> {
        let root = self.get_object(field, "gas")?;

        let root = match root.get(&self.parent.to_string()) {
            Some(Value::Object(m)) => m,
            Some(_) => {
                return Err(Self::err(
                    &self.path,
                    field,
                    format!("matching constants entry `{}` is not an object", field),
                ))
            }
            None => root,
        };

        let mut field_values = vec![];
        for (k, v) in root {
            let k: Ident = syn::parse_str(k).map_err(|e| {
                Self::err(
                    &self.path,
                    field,
                    format!("failed to parse key attribute `{}`: {}", k, e),
                )
            })?;

            let v = match v {
                Value::Array(a) => a
                    .iter()
                    .map(|v| match v {
                        Value::Bool(b) => Ok(*b as u64),
                        Value::Number(n) => n.as_u64().ok_or_else(|| {
                            Self::err(
                                &self.path,
                                field,
                                format!(
                                    "the value of the field `{k}` must be an array of valid `u64`"
                                ),
                            )
                        }),
                        _ => Err(Self::err(
                            &self.path,
                            field,
                            format!(
                            "the value of the field `{k}` must be an array of numbers, or booleans"
                        ),
                        )),
                    })
                    .collect::<Result<_, _>>()?,
                Value::Number(n) => n
                    .as_u64()
                    .ok_or_else(|| {
                        Self::err(
                            &self.path,
                            field,
                            format!("the value of the field `{k}` must be a `u64`"),
                        )
                    })
                    .map(|n| vec![n])?,
                Value::Bool(b) => vec![*b as u64],

                _ => {
                    return Err(Self::err(
                        &self.path,
                        field,
                        format!(
                            "the value of the field `{k}` must be an array, number, or boolean"
                        ),
                    ))
                }
            };

            field_values.push(quote::quote!(#k: <<<Self as ::sov_modules_api::Module>::Context as ::sov_modules_api::Context>::GasUnit as ::sov_modules_api::GasUnit>::from_arbitrary_dimensions(&[#(#v,)*])));
        }

        // remove generics, if any
        let mut ty = ty.clone();
        if let Type::Path(TypePath { path, .. }) = &mut ty {
            if let Some(p) = path.segments.last_mut() {
                p.arguments = PathArguments::None;
            }
        }

        Ok(quote::quote! {
            let #field = #ty {
                #(#field_values,)*
            };
        })
    }

    pub fn parse_constant(
        &self,
        ty: &Type,
        field: &Ident,
        vis: syn::Visibility,
        attrs: &[syn::Attribute],
    ) -> Result<TokenStream, syn::Error> {
        let root = self.get_object(field, "constants")?;
        let value = root.get(&field.to_string()).ok_or_else(|| {
            Self::err(
                &self.path,
                field,
                format!("manifest does not contain a `{}` attribute", field),
            )
        })?;
        let value = self.value_to_tokens(field, value, ty)?;

        if let Type::Reference(tr) = ty {
            if tr.lifetime.is_none() {
                let output = quote::quote! {
                    #(#attrs)*
                    #vis const #field: #ty = & #value;
                };
                return Ok(output);
            }
        }

        Ok(quote::quote! {
            #(#attrs)*
            #vis const #field: #ty = #value;
        })
    }

    fn value_to_tokens(
        &self,
        field: &Ident,
        value: &serde_json::Value,
        ty: &Type,
    ) -> Result<TokenStream, syn::Error> {
        match value {
            Value::Null => Err(Self::err(
                &self.path,
                field,
                format!("`{}` is `null`", field),
            )),
            Value::Bool(b) => Ok(quote::quote!(#b)),
            Value::Number(n) => {
                if n.is_u64() {
                    let n = n.as_u64().unwrap();
                    Ok(quote::quote!(#n as #ty))
                } else if n.is_i64() {
                    let n = n.as_i64().unwrap();
                    Ok(quote::quote!(#n as #ty))
                } else {
                    Err(Self::err(&self.path, field, "All numeric values must be representable as 64 bit integers during parsing.".to_string()))
                }
            }
            Value::String(s) => Ok(quote::quote!(#s)),
            Value::Array(arr) => {
                let mut values = Vec::with_capacity(arr.len());
                let ty = match ty {
                    Type::Array(ty) => &ty.elem,
                    Type::Reference(ty) => {
                        match ty.elem.as_ref() {
                            Type::Slice(ty) => &ty.elem,
                            _ => return Err(Self::err(
                                &self.path,
                                field,
                                format!(
                                    "Found value of type {:?} while parsing `{}` but expected a slice type ",
                                    ty, field
                                ),
                            )),
                        }
                    }
                    _ => return Err(Self::err(
                        &self.path,
                        field,
                        format!(
                            "Found value of type {:?} while parsing `{}` but expected an array type ",
                            ty, field
                        ),
                    ))
                };
                for (idx, value) in arr.iter().enumerate() {
                    values.push(self.value_to_tokens(
                        &format_ident!("{field}_{idx}"),
                        value,
                        ty,
                    )?);
                }
                Ok(quote::quote!([#(#values,)*]))
            }
            Value::Object(_) => todo!(),
        }
    }

    fn err<P, T>(path: P, ident: &syn::Ident, msg: T) -> syn::Error
    where
        P: AsRef<Path>,
        T: fmt::Display,
    {
        syn::Error::new(
            ident.span(),
            format!(
                "failed to parse manifest `{}` for `{}`: {}",
                path.as_ref().display(),
                ident,
                msg
            ),
        )
    }
}

#[test]
fn parse_gas_config_works() {
    let input = r#"{
        "comment": "Sovereign SDK constants",
        "gas": {
            "complex_math_operation": [1, 2, 3],
            "some_other_operation": [4, 5, 6]
        }
    }"#;

    let parent = Ident::new("Foo", proc_macro2::Span::call_site());
    let gas_config: Type = syn::parse_str("FooGasConfig<C::GasUnit>").unwrap();
    let field: Ident = syn::parse_str("foo_gas_config").unwrap();

    let decl = Manifest::read_str(input, PathBuf::from("foo.json"), &parent)
        .unwrap()
        .parse_gas_config(&gas_config, &field)
        .unwrap();

    #[rustfmt::skip]
    assert_eq!(
        decl.to_string(),
        quote::quote!(
            let foo_gas_config = FooGasConfig {
                complex_math_operation: <<<Self as ::sov_modules_api::Module>::Context as ::sov_modules_api::Context>::GasUnit as ::sov_modules_api::GasUnit>::from_arbitrary_dimensions(&[1u64, 2u64, 3u64, ]),
                some_other_operation: <<<Self as ::sov_modules_api::Module>::Context as ::sov_modules_api::Context>::GasUnit as ::sov_modules_api::GasUnit>::from_arbitrary_dimensions(&[4u64, 5u64, 6u64, ]),
            };
        )
        .to_string()
    );
}

#[test]
fn parse_gas_config_single_dimension_works() {
    let input = r#"{
        "comment": "Sovereign SDK constants",
        "gas": {
            "complex_math_operation": 1,
            "some_other_operation": 2
        }
    }"#;

    let parent = Ident::new("Foo", proc_macro2::Span::call_site());
    let gas_config: Type = syn::parse_str("FooGasConfig<C::GasUnit>").unwrap();
    let field: Ident = syn::parse_str("foo_gas_config").unwrap();

    let decl = Manifest::read_str(input, PathBuf::from("foo.json"), &parent)
        .unwrap()
        .parse_gas_config(&gas_config, &field)
        .unwrap();

    #[rustfmt::skip]
    assert_eq!(
        decl.to_string(),
        quote::quote!(
            let foo_gas_config = FooGasConfig {
                complex_math_operation: <<<Self as ::sov_modules_api::Module>::Context as ::sov_modules_api::Context>::GasUnit as ::sov_modules_api::GasUnit>::from_arbitrary_dimensions(&[1u64, ]),
                some_other_operation: <<<Self as ::sov_modules_api::Module>::Context as ::sov_modules_api::Context>::GasUnit as ::sov_modules_api::GasUnit>::from_arbitrary_dimensions(&[2u64, ]),
            };
        )
        .to_string()
    );
}
