// TODO remove once consumed
#![allow(dead_code)]

use std::path::PathBuf;
use std::{env, fmt, fs, ops};

use proc_macro2::TokenStream;
use toml::{Table, Value};

use crate::common::StructDef;
use crate::module_info::parsing::ModuleField;

#[derive(Debug, Clone)]
pub struct Manifest {
    path: PathBuf,
    table: Table,
}

impl ops::Deref for Manifest {
    type Target = Table;

    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl Manifest {
    /// The file name of the manifest.
    pub const MANIFEST_NAME: &'static str = "constants.toml";

    /// Reads a `sovereign.toml` manifest file, recursing from the target directory that builds the
    /// current implementation.
    ///
    /// If the environment variable `CONSTANTS_MANIFEST` is set, it will use that instead.
    pub fn read() -> anyhow::Result<Self> {
        let initial_path = match env::var("CONSTANTS_MANIFEST") {
            Ok(p) => PathBuf::from(&p).canonicalize().map_err(|e| {
                anyhow::anyhow!("failed access base dir for sovereign manifest file `{p}`: {e}",)
            }),
            Err(_) => {
                // read the target directory set via build script since `OUT_DIR` is available only at build
                let initial_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .canonicalize()
                    .map_err(|e| {
                        anyhow::anyhow!("failed access base dir for sovereign manifest file: {e}")
                    })?
                    .join("target-path");

                let initial_path = fs::read_to_string(&initial_path).map_err(|e| {
                    anyhow::anyhow!("failed to read target path for sovereign manifest file: {e}")
                })?;

                PathBuf::from(initial_path.trim())
                    .canonicalize()
                    .map_err(|e| {
                        anyhow::anyhow!("failed access base dir for sovereign manifest file: {e}")
                    })
            }
        }?;

        let path: PathBuf;
        let mut current_path = initial_path.as_path();
        loop {
            if current_path.join(Self::MANIFEST_NAME).exists() {
                path = current_path.join(Self::MANIFEST_NAME);
                break;
            }

            current_path = current_path.parent().ok_or_else(|| {
                anyhow::anyhow!("Could not find a parent {}", Self::MANIFEST_NAME)
            })?;
        }

        let manifest = fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Could not read the parent `{}`: {e}", path.display()))?;

        let table = toml::from_str(&manifest)
            .map_err(|e| anyhow::anyhow!("Could not parse `{}`: {}", path.display(), e))?;

        Ok(Self { path, table })
    }

    /// Parses a module struct from the manifest file. Returns a `TokenStream` with the following
    /// structure:
    ///
    /// ```rust,ignore
    /// let foo = Foo {
    ///     bar: Into::into("baz"),
    /// };
    /// ```
    ///
    /// Since this will be static code, the `TokensStream` will act as a constant, if there is no
    /// allocation such as with `String`.
    ///
    /// The routine will first query `section.parent`, and then fallback to `section`. Example
    ///
    /// ```toml
    /// [module]
    /// bar = "general"
    ///
    /// [module.Foo]
    /// bar = "baz"
    /// ```
    ///
    /// A call with a parent `Foo` will yield `bar = "baz"`, and a call with a parent `Etc` will
    /// yield `bar = "general"`.
    pub(crate) fn parse_module_struct(
        &self,
        section: &str,
        parent: &StructDef,
        field: &ModuleField,
    ) -> Result<TokenStream, syn::Error> {
        let root = self
            .table
            .get(section)
            .ok_or_else(|| self.err(&field.ident, format!("no `{}` section", section)))?
            .as_table()
            .ok_or_else(|| {
                self.err(
                    &field.ident,
                    format!("`{}` section must be a table", section),
                )
            })?
            .clone();

        let root = match root.get(&parent.ident.to_string()) {
            Some(Value::Table(t)) => t.clone(),
            _ => root
                .into_iter()
                // skip all tables so other modules are not included
                .filter(|(_, v)| !v.is_table())
                .collect(),
        };

        let struct_fields = root
            .iter()
            .map(|(f, v)| (quote::format_ident!("{f}"), v))
            .map(|(f, v)| match v {
                // TODO this can be optimized to specific cases based on type and avoid
                // `Into::into` (i.e. u32 as u64)
                Value::String(v) => Ok(quote::quote!(#f: #v)),
                Value::Integer(v) => Ok(quote::quote!(#f: #v)),
                Value::Float(v) => Ok(quote::quote!(#f: #v)),
                Value::Boolean(v) => Ok(quote::quote!(#f: #v)),
                _ => Err(self.err(
                    &field.ident,
                    format!(
                        "the contents of the section `{}` must be string, integer, float, or boolean",
                        section
                    ),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;

        let t = &field.ty;
        let field_ident = &field.ident;

        Ok(quote::quote! {
            let #field_ident = #t {
                #(#struct_fields,)*
            };
        })
    }

    fn err<T>(&self, ident: &syn::Ident, msg: T) -> syn::Error
    where
        T: fmt::Display,
    {
        syn::Error::new(
            ident.span(),
            format!(
                "failed to parse manifest `{}` for `{}`: {}",
                self.path.display(),
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
        .join(Manifest::MANIFEST_NAME)
        .canonicalize()
        .unwrap();

    let expected = fs::read_to_string(path).unwrap();
    let expected = toml::from_str(&expected).unwrap();

    let manifest = Manifest::read().unwrap();
    assert_eq!(*manifest, expected);
}
