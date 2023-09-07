use proc_macro2::{self, Ident, Span};
use syn::{
    Attribute, DataStruct, DeriveInput, ImplGenerics, PathArguments, TypeGenerics, WhereClause,
};

use self::parsing::{ModuleField, ModuleFieldAttribute, StructDef};
use crate::common::get_generics_type_param;

pub(crate) fn derive_module_info(
    input: DeriveInput,
) -> Result<proc_macro::TokenStream, syn::Error> {
    let struct_def = StructDef::parse(&input)?;

    let impl_prefix_functions = impl_prefix_functions(&struct_def)?;
    let impl_new = impl_module_info(&struct_def)?;

    Ok(quote::quote! {
        #impl_prefix_functions

        #impl_new
    }
    .into())
}

// Creates a prefix function for each field of the underlying structure.
fn impl_prefix_functions(struct_def: &StructDef) -> Result<proc_macro2::TokenStream, syn::Error> {
    let StructDef {
        ident,
        impl_generics,
        type_generics,
        fields,
        where_clause,
        ..
    } = struct_def;

    let prefix_functions = fields
        .iter()
        // Don't generate prefix functions for modules or addresses; only state.
        .filter(|field| matches!(field.attr, ModuleFieldAttribute::State { .. }))
        .map(|field| make_prefix_func(field, ident));

    Ok(quote::quote! {
        impl #impl_generics #ident #type_generics #where_clause{
            #(#prefix_functions)*
        }
    })
}

// Implements the `ModuleInfo` trait.
fn impl_module_info(struct_def: &StructDef) -> Result<proc_macro2::TokenStream, syn::Error> {
    let module_address = struct_def.module_address();

    let StructDef {
        ident,
        impl_generics,
        type_generics,
        generic_param,
        fields,
        where_clause,
    } = struct_def;

    let mut impl_self_init = Vec::default();
    let mut impl_self_body = Vec::default();
    let mut modules = Vec::default();

    for field in fields.iter() {
        match &field.attr {
            ModuleFieldAttribute::State { codec_builder } => {
                impl_self_init.push(make_init_state(
                    field,
                    &codec_builder
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(default_codec_builder),
                )?);
                impl_self_body.push(&field.ident);
            }
            ModuleFieldAttribute::Module => {
                impl_self_init.push(make_init_module(field)?);
                impl_self_body.push(&field.ident);
                modules.push(&field.ident);
            }
            ModuleFieldAttribute::Address => {
                impl_self_init.push(make_init_address(field, ident, generic_param)?);
                impl_self_body.push(&field.ident);
            }
        };
    }

    let fn_address = make_fn_address(&module_address.ident)?;
    let fn_dependencies = make_fn_dependencies(modules);
    let fn_prefix = make_module_prefix_fn(ident);

    Ok(quote::quote! {
        impl #impl_generics ::std::default::Default for #ident #type_generics #where_clause{

            fn default() -> Self {
                #(#impl_self_init)*

                Self{
                    #(#impl_self_body),*
                }
            }
        }

        impl #impl_generics ::sov_modules_api::ModuleInfo for #ident #type_generics #where_clause{
            type Context = #generic_param;

            #fn_prefix

            #fn_address

            #fn_dependencies
        }
    })
}

fn default_codec_builder() -> syn::Path {
    syn::parse_str("::core::default::Default::default").unwrap()
}

fn make_prefix_func(
    field: &ModuleField,
    module_ident: &proc_macro2::Ident,
) -> proc_macro2::TokenStream {
    let field_ident = &field.ident;
    let prefix_func_ident = prefix_func_ident(field_ident);

    // generates prefix functions:
    //   fn _prefix_field_ident() -> sov_modules_api::Prefix {
    //      let module_path = "some_module";
    //      sov_modules_api::Prefix::new_storage(module_path, module_name, field_ident)
    //   }
    quote::quote! {
        fn #prefix_func_ident() -> sov_modules_api::Prefix {
            let module_path = module_path!();
            sov_modules_api::Prefix::new_storage(module_path, stringify!(#module_ident), stringify!(#field_ident))
        }
    }
}

fn prefix_func_ident(ident: &proc_macro2::Ident) -> proc_macro2::Ident {
    syn::Ident::new(&format!("_prefix_{ident}"), ident.span())
}

fn make_fn_address(
    address_ident: &proc_macro2::Ident,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    Ok(quote::quote! {
        fn address(&self) -> &<Self::Context as ::sov_modules_api::Spec>::Address {
           &self.#address_ident
        }
    })
}

fn make_fn_dependencies(modules: Vec<&proc_macro2::Ident>) -> proc_macro2::TokenStream {
    let address_tokens = modules.iter().map(|ident| {
        quote::quote! {
            &self.#ident.address()
        }
    });

    quote::quote! {
        fn dependencies(&self) -> ::std::vec::Vec<&<Self::Context as sov_modules_api::Spec>::Address> {
            ::std::vec![#(#address_tokens),*]
        }
    }
}
fn make_init_state(
    field: &ModuleField,
    encoding_constructor: &syn::Path,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let prefix_fun = prefix_func_ident(&field.ident);
    let field_ident = &field.ident;
    let ty = &field.ty;

    let ty = match ty {
        syn::Type::Path(syn::TypePath { path, .. }) => {
            let mut segments = path.segments.clone();

            let last = segments
                .last_mut()
                .expect("Impossible happened! A type path has no segments");

            // Remove generics for the type SomeType<G> => SomeType
            last.arguments = PathArguments::None;
            segments
        }

        _ => {
            return Err(syn::Error::new_spanned(
                ty,
                "Type not supported by the `ModuleInfo` macro",
            ));
        }
    };

    // generates code for the state initialization:
    //  let state_prefix = Self::_prefix_field_ident().into();
    //  let field_ident = path::StateType::new(state_prefix);
    Ok(quote::quote! {
        let state_prefix = Self::#prefix_fun().into();
        let #field_ident = #ty::with_codec(state_prefix, #encoding_constructor());
    })
}

fn make_init_module(field: &ModuleField) -> Result<proc_macro2::TokenStream, syn::Error> {
    let field_ident = &field.ident;
    let ty = &field.ty;

    Ok(quote::quote! {
        let #field_ident = <#ty as ::std::default::Default>::default();
    })
}

fn make_module_prefix_fn(struct_ident: &Ident) -> proc_macro2::TokenStream {
    let body = make_module_prefix_fn_body(struct_ident);
    quote::quote! {
        fn prefix(&self) -> sov_modules_api::Prefix {
           #body
        }
    }
}

fn make_module_prefix_fn_body(struct_ident: &Ident) -> proc_macro2::TokenStream {
    quote::quote! {
        let module_path = module_path!();
        sov_modules_api::Prefix::new_module(module_path, stringify!(#struct_ident))
    }
}

fn make_init_address(
    field: &ModuleField,
    struct_ident: &Ident,
    generic_param: &Ident,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let field_ident = &field.ident;
    let generate_prefix = make_module_prefix_fn_body(struct_ident);

    Ok(quote::quote! {
        use ::sov_modules_api::digest::Digest as _;
        let prefix = {
            #generate_prefix
        };

        let #field_ident : <#generic_param as sov_modules_api::Spec>::Address =
            <#generic_param as ::sov_modules_api::Spec>::Address::try_from(&prefix.hash::<#generic_param>())
                .unwrap_or_else(|e| panic!("ModuleInfo macro error, unable to create an Address for module: {}", e));
    })
}

/// Internal `proc macro` parsing utilities.
pub mod parsing {
    use super::*;

    pub struct StructDef<'a> {
        pub ident: &'a proc_macro2::Ident,
        pub impl_generics: ImplGenerics<'a>,
        pub type_generics: TypeGenerics<'a>,
        pub generic_param: Ident,

        pub fields: Vec<ModuleField>,
        pub where_clause: Option<&'a WhereClause>,
    }

    impl<'a> StructDef<'a> {
        pub fn parse(input: &'a DeriveInput) -> syn::Result<Self> {
            let ident = &input.ident;
            let generic_param = get_generics_type_param(&input.generics, Span::call_site())?;
            let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
            let fields = parse_module_fields(&input.data)?;
            check_exactly_one_address(&fields)?;

            Ok(StructDef {
                ident,
                fields,
                impl_generics,
                type_generics,
                generic_param,
                where_clause,
            })
        }

        pub fn module_address(&self) -> &ModuleField {
            self.fields
                .iter()
                .find(|field| matches!(field.attr, ModuleFieldAttribute::Address))
                .expect("Module address not found but it was validated already; this is a bug")
        }
    }

    #[derive(Clone)]
    pub struct ModuleField {
        pub ident: syn::Ident,
        pub ty: syn::Type,
        pub attr: ModuleFieldAttribute,
    }

    #[derive(Clone)]
    pub enum ModuleFieldAttribute {
        Module,
        State { codec_builder: Option<syn::Path> },
        Address,
    }

    impl ModuleFieldAttribute {
        fn parse(attr: &Attribute) -> syn::Result<Self> {
            match attr.path.segments[0].ident.to_string().as_str() {
                "module" => {
                    if attr.tokens.is_empty() {
                        Ok(Self::Module)
                    } else {
                        Err(syn::Error::new_spanned(
                            attr,
                            "The `#[module]` attribute does not accept any arguments.",
                        ))
                    }
                }
                "address" => {
                    if attr.tokens.is_empty() {
                        Ok(Self::Address)
                    } else {
                        Err(syn::Error::new_spanned(
                            attr,
                            "The `#[address]` attribute does not accept any arguments.",
                        ))
                    }
                }
                "state" => parse_state_attr(attr),
                _ => unreachable!("attribute names were validated already; this is a bug"),
            }
        }
    }

    fn parse_state_attr(attr: &Attribute) -> syn::Result<ModuleFieldAttribute> {
        let syntax_err =
            syn::Error::new_spanned(attr, "Invalid syntax for the `#[state]` attribute.");

        let meta = if attr.tokens.is_empty() {
            return Ok(ModuleFieldAttribute::State {
                codec_builder: None,
            });
        } else {
            attr.parse_meta()?
        };

        let meta_list = match meta {
            syn::Meta::List(l) if !l.nested.is_empty() => l,
            _ => return Err(syntax_err),
        };
        let name_value = match &meta_list.nested[0] {
            syn::NestedMeta::Meta(syn::Meta::NameValue(nv)) => nv,
            _ => return Err(syntax_err),
        };

        if name_value.path.get_ident().map(Ident::to_string).as_deref() != Some("codec_builder") {
            return Err(syntax_err);
        }

        let codec_builder_path = match &name_value.lit {
            syn::Lit::Str(lit) => lit.parse_with(syn::Path::parse_mod_style)?,
            _ => return Err(syntax_err),
        };
        Ok(ModuleFieldAttribute::State {
            codec_builder: Some(codec_builder_path),
        })
    }

    fn parse_module_fields(data: &syn::Data) -> syn::Result<Vec<ModuleField>> {
        let data_struct = data_to_struct(data)?;
        let mut parsed_fields = vec![];

        for field in data_struct.fields.iter() {
            let ident = get_field_ident(field)?;
            let ty = field.ty.clone();
            let attr = get_field_attribute(field)?;

            parsed_fields.push(ModuleField {
                ident: ident.clone(),
                ty,
                attr: ModuleFieldAttribute::parse(attr)?,
            });
        }

        Ok(parsed_fields)
    }

    fn check_exactly_one_address(fields: &[ModuleField]) -> syn::Result<()> {
        let address_fields = fields
            .iter()
            .filter(|field| matches!(field.attr, ModuleFieldAttribute::Address))
            .collect::<Vec<_>>();

        match address_fields.len() {
            0 => Err(syn::Error::new(
                Span::call_site(),
                "The `ModuleInfo` macro requires `[address]` attribute.",
            )),
            1 => Ok(()),
            _ => Err(syn::Error::new_spanned(
                address_fields[1].ident.clone(),
                format!(
                    "The `address` attribute is defined more than once, revisit field: {}",
                    address_fields[1].ident,
                ),
            )),
        }
    }

    fn data_to_struct(data: &syn::Data) -> syn::Result<&DataStruct> {
        match data {
            syn::Data::Struct(data_struct) => Ok(data_struct),
            syn::Data::Enum(en) => Err(syn::Error::new_spanned(
                en.enum_token,
                "The `ModuleInfo` macro supports structs only.",
            )),
            syn::Data::Union(un) => Err(syn::Error::new_spanned(
                un.union_token,
                "The `ModuleInfo` macro supports structs only.",
            )),
        }
    }

    fn get_field_ident(field: &syn::Field) -> syn::Result<&syn::Ident> {
        field.ident.as_ref().ok_or(syn::Error::new_spanned(
            field,
            "The `ModuleInfo` macro supports structs only, unnamed fields witnessed.",
        ))
    }

    fn get_field_attribute(field: &syn::Field) -> syn::Result<&Attribute> {
        let ident = get_field_ident(field)?;
        let mut attr = None;
        for a in field.attrs.iter() {
            match a.path.segments[0].ident.to_string().as_str() {
                "state" | "module" | "address" => {
                    if attr.is_some() {
                        return Err(syn::Error::new_spanned(ident, "Only one attribute out of `#[module]`, `#[state]` and `#[address]` is allowed per field."));
                    } else {
                        attr = Some(a);
                    }
                }
                _ => {}
            }
        }

        if let Some(attr) = attr {
            Ok(attr)
        } else {
            Err(syn::Error::new_spanned(
                ident,
                "This field is missing an attribute: add `#[module]`, `#[state]` or `#[address]`.",
            ))
        }
    }
}
