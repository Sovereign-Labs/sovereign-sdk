use convert_case::{Case, Casing};
use darling::ast::NestedMeta;
use darling::util::{parse_attribute_to_meta_list, WithOriginal};
use darling::FromMeta;
use quote::ToTokens;
use syn::{Ident, Meta};

/// For parsing: list of attributes to not ignore
const PARSED_FOREIGN_ATTRS: [&str; 2] = ["serde", "borsh"];
/// For parsing: helper struct parsing a single attr
#[derive(Debug, FromMeta)]
#[allow(clippy::large_enum_variant)]
pub enum SingleForeignAttr {
    Serde(Serde),
    Borsh(Borsh),
}

/// Output structure collecting the parsed information from every foreign attribute we're handling
#[derive(Debug, Default, FromMeta)]
pub struct ForeignAttrs {
    pub serde: Serde,
    pub borsh: Borsh,
}

#[allow(dead_code)]
#[derive(Debug, Default, FromMeta)]
pub struct Borsh {
    pub use_discriminant: bool,
}

#[derive(Debug, Default, FromMeta)]
#[darling(allow_unknown_fields)]
pub struct Serde {
    /// used by serde on structs and variants
    rename_all: Option<WithOriginal<String, Meta>>,
    // TODO: these aren't currently used in the SDK and can be implemented later if needed
    // rename_all_fields: Option<String>, // used by serde on structs
    //rename: Option<String>, // used by serde on structs, variants and individual fields
}

/// This is an MVP implementation that handles the rename_all attribute, but not `rename`
/// attributes on fields.
/// To add support to `rename`, attribute parsing should also be extended to `InputField`s, and
/// then some extra logic needs to be added to construct a merged `SerdeRename` from the parent
/// type's one (where `rename_all = Some(...)`) and the one from the field (where `rename =
/// Some(...)`). The latter should override the former.
impl Serde {
    fn rename_using_rename_all(&self, original: &Ident) -> Result<String, syn::Error> {
        match &self.rename_all {
            Some(str) if str.parsed == "snake_case" => {
                Ok(original.to_string().to_case(Case::Snake))
            }
            Some(str) => Err(syn::Error::new_spanned(
                &str.original,
                "Rename_all option is unsupported in UniversalWallet",
            )),
            _ => Ok(original.to_string()),
        }
    }

    pub fn rename_field(&self, original: &Ident) -> Result<String, syn::Error> {
        self.rename_using_rename_all(original)
    }

    pub fn rename_variant(&self, original: &Ident) -> Result<String, syn::Error> {
        self.rename_using_rename_all(original)
    }

    pub fn rename_typename(&self, original: &Ident) -> Result<String, syn::Error> {
        Ok(original.to_string()) // TODO: implement the `rename` attribute
    }
}

pub fn parse_foreign_attrs(attrs: Vec<syn::Attribute>) -> darling::Result<ForeignAttrs> {
    let mut res = ForeignAttrs::default();
    for a in attrs.iter() {
        if a.path()
            .get_ident()
            .is_some_and(|p| PARSED_FOREIGN_ATTRS.contains(&p.to_string().as_str()))
        {
            let meta_list = parse_attribute_to_meta_list(a)?;
            let list = NestedMeta::parse_meta_list(meta_list.into_token_stream())?;
            let known_attr = SingleForeignAttr::from_list(&list)?;
            match known_attr {
                SingleForeignAttr::Serde(serde) => {
                    // TODO: better merging logic here if parsing multiple attributes
                    res.serde.rename_all = serde.rename_all;
                }
                SingleForeignAttr::Borsh(borsh) => {
                    res.borsh.use_discriminant = borsh.use_discriminant;
                }
            }
        }
    }
    Ok(res)
}
