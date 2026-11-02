use std::path::{Path, PathBuf};
use std::{fmt, fs};

use proc_macro2::{Ident, TokenStream};
use syn::{PathArguments, Type, TypePath};
use toml::Value;

const CONSTANTS_MANIFEST_PATH: Option<&str> = option_env!("CONSTANTS_MANIFEST_PATH");

/// Constants holding per-network chain metadata. When a `chain-metadata.toml`
/// file exists next to `constants.toml`, these keys are read from it instead
/// (see [`Manifest::read_for_constant`]), so editing them recompiles only the
/// crates that read them.
///
/// The routing is static — by key name only. A content-based "look in one
/// file, then the other" lookup would make the choice depend on both files'
/// contents, forcing every consumer to track both files and destroying the
/// fine-grained recompilation this split exists to provide.
const CHAIN_METADATA_KEYS: &[&str] = &[
    "CHAIN_ID",
    "CHAIN_NAME",
    "CHAIN_HASH_OVERRIDES",
    "BATCH_NAMESPACE",
    "PROOF_NAMESPACE",
];

fn is_chain_metadata_key(name: &Ident) -> bool {
    let name = name.to_string();
    CHAIN_METADATA_KEYS.contains(&name.as_str())
}

/// Path of the chain-metadata manifest sitting next to the given constants
/// manifest: `chain-metadata.toml` next to `constants.toml`, or
/// `chain-metadata.testing.toml` next to `constants.testing.toml` (the baked
/// constants filename already encodes whether build.rs selected test mode, so
/// there is deliberately no cross-variant fallback).
fn chain_metadata_sibling(constants_path: &Path) -> PathBuf {
    let filename = if constants_path.file_name().and_then(|name| name.to_str())
        == Some("constants.testing.toml")
    {
        "chain-metadata.testing.toml"
    } else {
        "chain-metadata.toml"
    };
    constants_path.with_file_name(filename)
}

#[derive(Debug, Clone)]
pub struct Manifest<'a> {
    parent: &'a Ident,
    path: PathBuf,
    value: Value,
    /// Whether `path` is a real file that generated code must declare as a
    /// compile-time dependency (see [`Self::dependency_tracking_tokens`]).
    /// `false` for manifests parsed from in-memory strings (unit tests).
    track_as_dependency: bool,
}

impl<'a> Manifest<'a> {
    /// Parse a manifest file from a string.
    ///
    /// The provided path will be used to feedback error to the user, if any.
    ///
    /// The `parent` is used to report the errors to the correct span location.
    pub fn read_str<S>(manifest: S, path: PathBuf, parent: &'a Ident) -> syn::Result<Self>
    where
        S: AsRef<str>,
    {
        let value = toml::from_str(manifest.as_ref())
            .map_err(|e| Self::err(&path, parent, format!("failed to parse manifest: {e}")))?;

        Ok(Self {
            parent,
            path,
            value,
            track_as_dependency: false,
        })
    }

    /// Reads a `constants.toml` manifest file, walking up the directory tree
    /// starting from
    /// [`OUT_DIR`](https://doc.rust-lang.org/cargo/reference/environment-variables.html) until it finds
    /// one.
    ///
    /// If the environment variable `CONSTANTS_MANIFEST` is set, the file will
    /// be read from that directory instead.
    ///
    /// If the `test` Cargo feature is enabled or the environment variable
    /// `SOV_TEST_MODE_CONST_MANIFEST` is set, the proc-macro will look for a
    /// file named `constants.testing.toml` instead.
    ///
    /// # Arguments
    ///
    /// `parent` is used to report the errors to the correct span location.
    pub fn read_constants(parent: &'a Ident) -> syn::Result<Self> {
        let constants_path = Self::baked_constants_path(parent)?;
        Self::read_file(constants_path, parent)
    }

    /// Reads the manifest that serves the constant `name`, for
    /// `config_value!`-style lookups.
    ///
    /// Most constants live in `constants.toml`, resolved exactly like
    /// [`Self::read_constants`]. The chain metadata keys listed in
    /// [`CHAIN_METADATA_KEYS`] are instead served by the
    /// [`chain_metadata_sibling`] file, when it exists:
    ///
    /// * chain-metadata file exists → the key **must** be defined there;
    ///   a chain key missing from it is a hard error even if
    ///   `constants.toml` still defines it, because a partial migration
    ///   must fail loudly rather than silently make consumers track both
    ///   files.
    /// * chain-metadata file does not exist → the key is read from
    ///   `constants.toml` like any other constant, so pre-split layouts
    ///   keep working unchanged.
    ///
    /// Whichever file is actually read becomes this manifest's tracked
    /// dependency (see [`Self::dependency_tracking_tokens`]), which is what
    /// makes editing chain metadata recompile only the crates that read it.
    pub fn read_for_constant(name: &'a Ident) -> syn::Result<Self> {
        let constants_path = Self::baked_constants_path(name)?;
        Self::read_for_constant_at(constants_path, name)
    }

    /// Path-parameterized body of [`Self::read_for_constant`], testable
    /// without the baked `CONSTANTS_MANIFEST_PATH`.
    fn read_for_constant_at(constants_path: PathBuf, name: &'a Ident) -> syn::Result<Self> {
        if is_chain_metadata_key(name) {
            let chain_metadata_path = chain_metadata_sibling(&constants_path);
            if chain_metadata_path.is_file() {
                let manifest = Self::read_file(chain_metadata_path, name)?;
                manifest.check_has_constant(name)?;
                return Ok(manifest);
            }
        }
        Self::read_file(constants_path, name)
    }

    /// The `constants.toml` path resolved by the build script (see
    /// `build.rs`) and baked into this proc-macro binary.
    fn baked_constants_path(parent: &Ident) -> syn::Result<PathBuf> {
        CONSTANTS_MANIFEST_PATH.map(PathBuf::from).ok_or_else(|| {
            syn::Error::new(
                parent.span(),
                format!(
                    "Failed to find a `{}` file in the current directory or any parent directory",
                    "constants.toml"
                ),
            )
        })
    }

    /// Reads and parses a manifest file, marking it as a tracked dependency
    /// of the consuming crate.
    fn read_file(path: PathBuf, parent: &'a Ident) -> syn::Result<Self> {
        let contents = fs::read_to_string(&path).map_err(|e| {
            Self::err(
                &path,
                parent,
                format!(
                    "failed to read `{}`: {}. The path is resolved once when \
                     `sov-modules-macros` is compiled; if the file was moved or the workspace \
                     was relocated since, run `cargo clean -p sov-modules-macros` to re-resolve \
                     it, or set the `CONSTANTS_MANIFEST` environment variable to the directory \
                     containing the file",
                    path.display(),
                    e
                ),
            )
        })?;

        let mut manifest = Self::read_str(contents, path, parent)?;
        manifest.track_as_dependency = true;
        Ok(manifest)
    }

    /// Errors unless `name` is defined under this manifest's `[constants]`
    /// table. Used to fail loudly on partially migrated chain-metadata
    /// files instead of falling back to `constants.toml`.
    fn check_has_constant(&self, name: &Ident) -> syn::Result<()> {
        let is_present = self
            .value
            .as_table()
            .and_then(|root| root.get("constants"))
            .and_then(toml::Value::as_table)
            .is_some_and(|constants| constants.contains_key(&name.to_string()));
        if is_present {
            return Ok(());
        }
        Err(syn::Error::new(
            name.span(),
            format!(
                "`{}` is a chain-metadata constant; it must be defined in `{}` \
                 (found the file but not the key). Move it there from `constants.toml`.",
                name,
                self.path.display(),
            ),
        ))
    }

    /// Tokens that record the manifest file in the dep-info of the crate whose
    /// macro expansion is currently being generated, so Cargo recompiles
    /// exactly the crates that read constants when the file changes.
    ///
    /// The `sov-modules-macros` build script intentionally does not watch the
    /// file (see `build.rs`); without these tokens, edits to the manifest
    /// would not trigger any recompilation at all.
    pub fn dependency_tracking_tokens(&self) -> TokenStream {
        if !self.track_as_dependency {
            return TokenStream::new();
        }
        // File-backed paths originate from the `CONSTANTS_MANIFEST_PATH` env
        // var, which is always valid UTF-8.
        let path = self
            .path
            .to_str()
            .expect("constants manifest path is not valid UTF-8");
        quote::quote!(
            const _: &[u8] = ::core::include_bytes!(#path);
        )
    }

    /// Gets the requested object from the manifest by key
    fn get_object(&self, field: &Ident, key: &str) -> syn::Result<&toml::Table> {
        self.value
            .as_table()
            .ok_or_else(|| Self::err(&self.path, field, "manifest is not an object"))?
            .get(key)
            .ok_or_else(|| {
                Self::err(
                    &self.path,
                    field,
                    format!("manifest does not contain a `{key}` attribute"),
                )
            })?
            .as_table()
            .ok_or_else(|| {
                Self::err(
                    &self.path,
                    field,
                    format!("`{key}` attribute of `{field}` is not a table"),
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
    /// Where `foo` and `bar` are fields of the TOML constants file under the located `gas` field.
    ///
    /// The `gas` field resolution will first attempt to query `gas.parent`, and then fallback to
    /// `gas`. They must be objects with arrays of integers as fields.
    pub fn parse_gas_config(&self, ty: &Type, field: &Ident) -> syn::Result<TokenStream> {
        let root = self.get_object(field, "gas")?;

        let root = match root.get(&self.parent.to_string()) {
            Some(Value::Table(t)) => t,
            Some(_) => {
                return Err(Self::err(
                    &self.path,
                    field,
                    format!("matching constants entry `{field}` is not an object"),
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
                    format!("failed to parse key attribute `{k}`: {e}"),
                )
            })?;

            let v = match v {
                Value::Array(a) => a
                    .iter()
                    .map(|v| match v {
                        Value::Boolean(b) => Ok(u64::from(*b)),
                        Value::Integer(n) => Ok(u64::try_from(*n).map_err(|_| {
                            Self::err(
                                &self.path,
                                field,
                                format!(
                                    "the value of the field `{k}` must be an array of valid `u64`"
                                ),
                            )
                        })?),
                        _ => Err(Self::err(
                            &self.path,
                            field,
                            format!(
                            "the value of the field `{k}` must be an array of numbers, or booleans"
                        ),
                        )),
                    })
                    .collect::<Result<_, _>>()?,
                Value::Integer(n) => vec![u64::try_from(*n).map_err(|_| {
                    Self::err(
                        &self.path,
                        field,
                        format!("the value of the field `{k}` must be a `u64`"),
                    )
                })?],
                Value::Boolean(b) => vec![u64::from(*b)],

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

            field_values.push(
                quote::quote!(#k:
                    {
                        #[cfg(feature = "gas-constant-estimation")]
                        {
                            <<Self as ::sov_modules_api::Module>::Spec as ::sov_modules_api::Spec>::Gas::from([#(#v,)*]).with_name(stringify!(#k).to_string().to_uppercase())
                        }
                        #[cfg(not(feature = "gas-constant-estimation"))]
                        {
                            <<Self as ::sov_modules_api::Module>::Spec as ::sov_modules_api::Spec>::Gas::from([#(#v,)*])
                        }
                    }
                ));
        }

        // remove generics, if any
        let mut ty = ty.clone();
        if let Type::Path(TypePath { path, .. }) = &mut ty {
            if let Some(p) = path.segments.last_mut() {
                p.arguments = PathArguments::None;
            }
        }

        let tracking = self.dependency_tracking_tokens();
        Ok(quote::quote! {
            #tracking
            let #field = #ty {
                #(#field_values,)*
            };
        })
    }

    pub fn get(&self, field: &Ident) -> syn::Result<&toml::Value> {
        let root = self.get_object(field, "constants")?;
        root.get(&field.to_string()).ok_or_else(|| {
            Self::err(
                &self.path,
                field,
                format!("manifest does not contain a `{field}` attribute"),
            )
        })
    }

    fn err<P, T>(path: P, ident: &Ident, msg: T) -> syn::Error
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gas_config_works() {
        let input = r#"
            [gas]
            complex_math_operation = [1, 2]
            some_other_operation = [4, 5]
        "#;

        let parent = Ident::new("Foo", proc_macro2::Span::call_site());
        let gas_config: Type = syn::parse_str("FooGasConfig<S::Gas>").unwrap();
        let field: Ident = syn::parse_str("foo_gas_config").unwrap();

        let decl = Manifest::read_str(input, PathBuf::from("foo.toml"), &parent)
            .unwrap()
            .parse_gas_config(&gas_config, &field)
            .unwrap();

        #[rustfmt::skip]
        assert_eq!(
            decl.to_string(),
            quote::quote!(
                let foo_gas_config = FooGasConfig {
                    complex_math_operation: {
                        #[cfg(feature = "gas-constant-estimation")]
                        {
                            <<Self as ::sov_modules_api::Module>::Spec as ::sov_modules_api::Spec>::Gas::from([1u64, 2u64, ]).with_name(stringify!(complex_math_operation).to_string().to_uppercase())
                        }
                        #[cfg(not(feature = "gas-constant-estimation"))]
                        {
                            <<Self as ::sov_modules_api::Module>::Spec as ::sov_modules_api::Spec>::Gas::from([1u64, 2u64, ])
                        }
                    },
                    some_other_operation: {
                        #[cfg(feature = "gas-constant-estimation")]
                        {
                            <<Self as ::sov_modules_api::Module>::Spec as ::sov_modules_api::Spec>::Gas::from([4u64, 5u64, ]).with_name(stringify!(some_other_operation).to_string().to_uppercase())
                        }
                        #[cfg(not(feature = "gas-constant-estimation"))]
                        {
                            <<Self as ::sov_modules_api::Module>::Spec as ::sov_modules_api::Spec>::Gas::from([4u64, 5u64, ])
                        }
                    },
                };
            )
            .to_string()
        );
    }

    fn ident(name: &str) -> Ident {
        Ident::new(name, proc_macro2::Span::call_site())
    }

    /// `constants.toml` path of a committed fixture layout under
    /// `tests/manifest_fixtures/`.
    fn fixture_constants_path(layout: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/manifest_fixtures")
            .join(layout)
            .join("constants.toml")
    }

    #[test]
    fn chain_metadata_sibling_of_production_manifest() {
        assert_eq!(
            chain_metadata_sibling(Path::new("/workspace/constants.toml")),
            PathBuf::from("/workspace/chain-metadata.toml")
        );
    }

    #[test]
    fn chain_metadata_sibling_of_testing_manifest() {
        assert_eq!(
            chain_metadata_sibling(Path::new("/workspace/constants.testing.toml")),
            PathBuf::from("/workspace/chain-metadata.testing.toml")
        );
    }

    #[test]
    fn chain_key_is_read_from_chain_metadata_file_when_present() {
        let field = ident("CHAIN_ID");
        let manifest =
            Manifest::read_for_constant_at(fixture_constants_path("split"), &field).unwrap();

        assert_eq!(
            manifest.path,
            fixture_constants_path("split").with_file_name("chain-metadata.toml"),
            "chain keys must be served by (and tracked against) the chain-metadata file"
        );
    }

    #[test]
    fn non_chain_key_is_read_from_constants_file() {
        let field = ident("MAX_TX_SIZE");
        let manifest =
            Manifest::read_for_constant_at(fixture_constants_path("split"), &field).unwrap();

        assert_eq!(
            manifest.path,
            fixture_constants_path("split"),
            "non-chain keys must always be served by constants.toml"
        );
    }

    #[test]
    fn chain_key_falls_back_to_constants_file_when_chain_metadata_is_absent() {
        let field = ident("CHAIN_ID");
        let manifest =
            Manifest::read_for_constant_at(fixture_constants_path("legacy"), &field).unwrap();

        assert_eq!(
            manifest.path,
            fixture_constants_path("legacy"),
            "pre-split layouts must keep reading chain keys from constants.toml"
        );
    }

    #[test]
    fn chain_key_missing_from_existing_chain_metadata_file_is_an_error() {
        let field = ident("CHAIN_ID");
        let error = Manifest::read_for_constant_at(fixture_constants_path("partial"), &field)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("`CHAIN_ID` is a chain-metadata constant"),
            "error must name the missing key: {error}"
        );
        assert!(
            error.contains("chain-metadata.toml"),
            "error must name the chain-metadata file: {error}"
        );
    }
}
