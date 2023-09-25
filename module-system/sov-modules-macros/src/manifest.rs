// TODO remove once consumed
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::{env, fmt, fs, ops};

use proc_macro2::{Ident, TokenStream};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Manifest {
    path: PathBuf,
    value: Value,
}

impl ops::Deref for Manifest {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl Manifest {
    /// Parse a manifest file from a string.
    ///
    /// The provided path will be used to feedback error to the user, if any.
    ///
    /// The `parent` is used to report the errors to the correct span location.
    pub fn read_str<S>(manifest: S, path: PathBuf, parent: &Ident) -> Result<Self, syn::Error>
    where
        S: AsRef<str>,
    {
        let value = serde_json::from_str(manifest.as_ref())
            .map_err(|e| Self::err(&path, parent, format!("failed to parse manifest: {e}")))?;

        Ok(Self { path, value })
    }

    /// Reads a `constants.json` manifest file, recursing from the target directory that builds the
    /// current implementation.
    ///
    /// If the environment variable `CONSTANTS_MANIFEST` is set, it will use that instead.
    ///
    /// The `parent` is used to report the errors to the correct span location.
    pub fn read_constants(parent: &Ident) -> Result<Self, syn::Error> {
        let manifest = "constants.json";
        let initial_path = match env::var("CONSTANTS_MANIFEST") {
            Ok(p) => PathBuf::from(&p).canonicalize().map_err(|e| {
                Self::err(
                    &p,
                    parent,
                    format!("failed access base dir for sovereign manifest file `{p}`: {e}"),
                )
            }),
            Err(_) => {
                // read the target directory set via build script since `OUT_DIR` is available only at build
                let initial_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .canonicalize()
                    .map_err(|e| {
                        Self::err(
                            manifest,
                            parent,
                            format!("failed access base dir for sovereign manifest file: {e}"),
                        )
                    })?
                    .join("target-path");

                let initial_path = fs::read_to_string(&initial_path).map_err(|e| {
                    Self::err(
                        &initial_path,
                        parent,
                        format!("failed to read target path for sovereign manifest file: {e}"),
                    )
                })?;

                PathBuf::from(initial_path.trim())
                    .canonicalize()
                    .map_err(|e| {
                        Self::err(
                            &initial_path,
                            parent,
                            format!("failed access base dir for sovereign manifest file: {e}"),
                        )
                    })
            }
        }?;

        let path: PathBuf;
        let mut current_path = initial_path.as_path();
        loop {
            if current_path.join(manifest).exists() {
                path = current_path.join(manifest);
                break;
            }

            current_path = current_path.parent().ok_or_else(|| {
                Self::err(
                    &current_path,
                    parent,
                    format!("Could not find a parent `{manifest}`"),
                )
            })?;
        }

        let manifest = fs::read_to_string(&path)
            .map_err(|e| Self::err(&current_path, parent, format!("failed to read file: {e}")))?;

        Self::read_str(manifest, path, parent)
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
    pub fn parse_gas_config(
        &self,
        parent: &Ident,
    ) -> Result<(Ident, TokenStream, TokenStream), syn::Error> {
        let root = self
            .value
            .as_object()
            .ok_or_else(|| Self::err(&self.path, parent, "manifest is not an object"))?
            .get("gas")
            .ok_or_else(|| {
                Self::err(
                    &self.path,
                    parent,
                    "manifest does not contain a `gas` attribute",
                )
            })?
            .as_object()
            .ok_or_else(|| {
                Self::err(
                    &self.path,
                    parent,
                    format!("`gas` attribute of `{}` is not an object", parent),
                )
            })?;

        let root = match root.get(&parent.to_string()) {
            Some(Value::Object(m)) => m,
            Some(_) => {
                return Err(Self::err(
                    &self.path,
                    parent,
                    format!(
                        "matching constants entry `{}` is not an object",
                        &parent.to_string()
                    ),
                ))
            }
            None => root,
        };

        let mut field_values = vec![];
        let mut fields = vec![];
        for (k, v) in root {
            let k: Ident = syn::parse_str(k).map_err(|e| {
                Self::err(
                    &self.path,
                    parent,
                    format!("failed to parse key attribyte `{}`: {}", k, e),
                )
            })?;

            let v = v
                .as_array()
                .ok_or_else(|| {
                    Self::err(
                        &self.path,
                        parent,
                        format!("`{}` attribute is not an array", k),
                    )
                })?
                .iter()
                .map(|v| {
                    v.as_u64().ok_or_else(|| {
                        Self::err(
                            &self.path,
                            parent,
                            format!("`{}` attribute is not an array of integers", k),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            let n = v.len();
            fields.push(quote::quote!(pub #k: [u64; #n]));
            field_values.push(quote::quote!(#k: [#(#v,)*]));
        }

        let ty = format!("{parent}GasConfig");
        let ty = syn::parse_str(&ty).map_err(|e| {
            Self::err(
                &self.path,
                parent,
                format!("failed to parse type name `{}`: {}", ty, e),
            )
        })?;

        let def = quote::quote! {
            #[allow(missing_docs)]
            #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct #ty {
                #(#fields,)*
            }
        };

        let decl = quote::quote! {
            #ty {
                #(#field_values,)*
            }
        };

        Ok((ty, def, decl))
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
fn fetch_manifest_works() {
    let path = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(path)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("constants.json")
        .canonicalize()
        .unwrap();

    let expected = fs::read_to_string(path).unwrap();
    let expected: Value = serde_json::from_str(&expected).unwrap();

    let parent = Ident::new("foo", proc_macro2::Span::call_site());
    let manifest = Manifest::read_constants(&parent).unwrap();
    assert_eq!(*manifest, expected);
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
    let (ty, def, decl) = Manifest::read_str(input, PathBuf::from("foo.json"), &parent)
        .unwrap()
        .parse_gas_config(&parent)
        .unwrap();

    #[rustfmt::skip]
    assert_eq!(
        ty.to_string(),
        quote::quote!(
            FooGasConfig
        )
        .to_string()
    );

    #[rustfmt::skip]
    assert_eq!(
        def.to_string(),
        quote::quote!(
            #[allow(missing_docs)]
            #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct FooGasConfig {
                pub complex_math_operation: [u64; 3usize],
                pub some_other_operation: [u64; 3usize],
            }
        )
        .to_string()
    );

    #[rustfmt::skip]
    assert_eq!(
        decl.to_string(),
        quote::quote!(
            FooGasConfig {
                complex_math_operation: [1u64, 2u64, 3u64, ],
                some_other_operation: [4u64, 5u64, 6u64, ],
            }
        )
        .to_string()
    );
}
