use proc_macro2::{self, Ident, Span};
use quote::ToTokens;
use syn::parse::Parse;
use syn::{
    Attribute, DataStruct, DeriveInput, ImplGenerics, PathArguments, TypeGenerics, WhereClause,
};

use self::parsing::{ModuleField, ModuleFieldAttribute, StructDef};
use crate::common::get_generics_type_param;
use crate::manifest::Manifest;

pub(crate) fn derive_module_info(input: &DeriveInput) -> syn::Result<proc_macro::TokenStream> {
    let struct_def = StructDef::parse(input)?;

    let impl_prefix_functions = impl_prefix_functions(&struct_def);
    let impl_new = impl_module_info(&struct_def)?;

    Ok(quote::quote! {
        #impl_prefix_functions

        #impl_new
    }
    .into())
}

// Creates a prefix function for each field of the underlying structure.
fn impl_prefix_functions(struct_def: &StructDef) -> proc_macro2::TokenStream {
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

    quote::quote! {
        impl #impl_generics #ident #type_generics #where_clause{
            #(#prefix_functions)*
        }
    }
}

// Implements the `ModuleInfo` trait.
fn impl_module_info(struct_def: &StructDef) -> syn::Result<proc_macro2::TokenStream> {
    let module_id = struct_def.module_id();

    let StructDef {
        ident,
        impl_generics,
        type_generics,
        generic_param,
        fields,
        where_clause,
        sequencer_safety_fn,
    } = struct_def;

    let mut impl_self_init = Vec::default();
    let mut impl_self_body = Vec::default();
    let mut modules = Vec::default();

    for field in fields {
        match &field.attr {
            ModuleFieldAttribute::State { codec_builder } => {
                impl_self_init.push(make_init_state(
                    field,
                    &codec_builder.clone().unwrap_or_else(default_codec_builder),
                )?);
                impl_self_body.push(&field.ident);
            }
            ModuleFieldAttribute::Module => {
                impl_self_init.push(make_init_module(field));
                impl_self_body.push(&field.ident);
                modules.push(&field.ident);
            }
            ModuleFieldAttribute::Address => {
                impl_self_init.push(make_init_id(field, ident, generic_param));
                impl_self_body.push(&field.ident);
            }
            ModuleFieldAttribute::Gas => {
                impl_self_init.push(make_init_gas_config(ident, field)?);
                impl_self_body.push(&field.ident);
            }
            ModuleFieldAttribute::Phantom => {
                impl_self_init.push(make_init_phantomdata(field));
                impl_self_body.push(&field.ident);
            }
        };
    }

    let fn_id = make_fn_id(&module_id.ident);
    let fn_dependencies = make_fn_dependencies(&modules);
    let fn_prefix = make_module_prefix_fn(ident);
    let fn_is_safe_for_sequencer = make_sequencer_safety_fn(sequencer_safety_fn);

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
            type Spec = #generic_param;

            #fn_prefix

            #fn_id

            #fn_is_safe_for_sequencer

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
    //   fn _prefix_field_ident() -> sov_modules_api::ModulePrefix {
    //      let module_path = "some_module";
    //      sov_modules_api::ModulePrefix::new_storage(module_path, module_name, field_ident)
    //   }
    quote::quote! {
        fn #prefix_func_ident() -> sov_modules_api::ModulePrefix {
            let module_path = module_path!();
            sov_modules_api::ModulePrefix::new_storage(module_path, stringify!(#module_ident), stringify!(#field_ident))
        }
    }
}

fn prefix_func_ident(ident: &Ident) -> Ident {
    Ident::new(&format!("_prefix_{ident}"), ident.span())
}

fn make_fn_id(id_ident: &proc_macro2::Ident) -> proc_macro2::TokenStream {
    quote::quote! {
        fn id(&self) -> &::sov_modules_api::ModuleId {
           &self.#id_ident
        }
    }
}

fn make_fn_dependencies(modules: &[&proc_macro2::Ident]) -> proc_macro2::TokenStream {
    let address_tokens = modules.iter().map(|ident| {
        quote::quote! {
            self.#ident.id()
        }
    });

    quote::quote! {
        fn dependencies(&self) -> ::std::vec::Vec<&::sov_modules_api::ModuleId> {
            ::std::vec![#(#address_tokens),*]
        }
    }
}
fn make_init_state(
    field: &ModuleField,
    encoding_constructor: &syn::Path,
) -> syn::Result<proc_macro2::TokenStream> {
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

fn make_init_module(field: &ModuleField) -> proc_macro2::TokenStream {
    let field_ident = &field.ident;
    let ty = &field.ty;
    let trait_assertion = quote::quote! { let _: <#ty as ::sov_modules_api::Module>::Spec; };

    quote::quote! {
        // Ensure that the type implements "Module" or "KernelModule" at compile time
        #trait_assertion
        let #field_ident = <#ty as ::std::default::Default>::default();
    }
}

fn make_init_gas_config(
    parent: &proc_macro2::Ident,
    field: &ModuleField,
) -> syn::Result<proc_macro2::TokenStream> {
    let field_ident = &field.ident;
    let ty = &field.ty;

    Manifest::read_constants(parent)?.parse_gas_config(ty, field_ident)
}

fn make_init_phantomdata(field: &ModuleField) -> proc_macro2::TokenStream {
    let field_ident = &field.ident;
    quote::quote! { let #field_ident = ::std::marker::PhantomData; }
}

fn make_module_prefix_fn(struct_ident: &proc_macro2::Ident) -> proc_macro2::TokenStream {
    let body = make_module_prefix_fn_body(struct_ident);
    quote::quote! {
        fn prefix(&self) -> sov_modules_api::ModulePrefix {
           #body
        }
    }
}

fn make_module_prefix_fn_body(struct_ident: &proc_macro2::Ident) -> proc_macro2::TokenStream {
    quote::quote! {
        let module_path = module_path!();
        sov_modules_api::ModulePrefix::new_module(module_path, stringify!(#struct_ident))
    }
}

fn make_init_id(
    field: &ModuleField,
    struct_ident: &proc_macro2::Ident,
    generic_param: &proc_macro2::Ident,
) -> proc_macro2::TokenStream {
    let field_ident = &field.ident;
    let generate_prefix = make_module_prefix_fn_body(struct_ident);

    quote::quote! {
        use ::sov_modules_api::digest::Digest as _;
        use ::sov_modules_api::ModuleId;
        let prefix = {
            #generate_prefix
        };

        let #field_ident : ModuleId =
            prefix.hash::<#generic_param>().into();
    }
}

fn make_sequencer_safety_fn(sequencer_safety_fn: &Option<syn::Path>) -> proc_macro2::TokenStream {
    if let Some(path) = sequencer_safety_fn {
        quote::quote! {
            fn is_safe_for_sequencer<'a>(&self, call: ::sov_modules_api::InnerEnumVariant<'a>, sequencer: &<<Self::Spec as ::sov_modules_api::Spec>::Da as ::sov_modules_api::DaSpec>::Address) -> bool {
                #path (self, call, sequencer)
            }
        }
    } else {
        quote::quote! {}
    }
}

/// Internal `proc macro` parsing utilities.
pub mod parsing {
    use proc_macro2::Ident;

    use super::*;

    /// A fully parsed and validated `struct` marked with
    /// `#[derive(Clone, ModuleInfo)]`.
    pub struct StructDef<'a> {
        pub ident: &'a Ident,
        pub impl_generics: ImplGenerics<'a>,
        pub type_generics: TypeGenerics<'a>,
        pub generic_param: Ident,

        pub fields: Vec<ModuleField>,
        pub where_clause: Option<&'a WhereClause>,
        pub sequencer_safety_fn: Option<syn::Path>,
    }

    impl<'a> StructDef<'a> {
        pub fn parse(input: &'a DeriveInput) -> syn::Result<Self> {
            let ident = &input.ident;
            let generic_param = get_generics_type_param(&input.generics, Span::call_site())?;
            let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
            let fields = parse_module_fields(&input.data)?;
            check_exactly_one_address(&fields)?;
            check_zero_or_one_gas(&fields)?;
            let mut sequencer_safety_fn = None;
            if let Some(attr) = input
                .attrs
                .iter()
                .find(|attr| attr.path().is_ident("module_info"))
            {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("sequencer_safety") {
                        let value = meta.value()?;
                        let str_contents: syn::LitStr = Parse::parse(value)?;
                        sequencer_safety_fn = Some(str_contents.parse()?);
                        Ok(())
                    } else {
                        Err(syn::Error::new_spanned(
                            attr,
                            format!(
                                r#"Unrecognized attribute #[module_info({})]`. Use #[module_info(sequencer_safety = "...")]"#,
                                meta.path.to_token_stream(),
                            ),
                        ))
                    }
                })?;
            };

            Ok(StructDef {
                ident,
                impl_generics,
                type_generics,
                generic_param,
                fields,
                where_clause,
                sequencer_safety_fn,
            })
        }

        pub fn module_id(&self) -> &ModuleField {
            self.fields
                .iter()
                .find(|field| matches!(field.attr, ModuleFieldAttribute::Address))
                .expect("Module id not found but it was validated already; this is a bug")
        }
    }

    #[derive(Clone)]
    pub struct ModuleField {
        pub ident: Ident,
        pub ty: syn::Type,
        pub attr: ModuleFieldAttribute,
    }

    #[derive(Clone)]
    pub enum ModuleFieldAttribute {
        Module,
        State { codec_builder: Option<syn::Path> },
        Address,
        Gas,
        Phantom,
    }

    impl ModuleFieldAttribute {
        fn parse(attr: &Attribute) -> syn::Result<Self> {
            let attr_name = attr
                .path()
                .segments
                .last()
                .ok_or_else(|| syn::Error::new_spanned(attr, "Invalid attribute"))?
                .ident
                .to_string();
            match attr_name.as_str() {
                "module" | "id" | "gas" | "phantom" => {
                    match &attr.meta {
                        syn::Meta::Path(_) => {
                            // This is the case for attributes without arguments
                            Ok(match attr_name.as_str() {
                                "module" => Self::Module,
                                "id" => Self::Address,
                                "gas" => Self::Gas,
                                "phantom" => Self::Phantom,
                                _ => unreachable!("attributes names should have been prefiltered by pattern; this is a bug"),
                            })
                        }
                        _ => Err(syn::Error::new_spanned(
                            attr,
                            format!(
                                "The `#[{attr_name}]` attribute does not accept any arguments."
                            ),
                        )),
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

        match &attr.meta {
            syn::Meta::Path(_) => {
                // This handles the case of a simple #[state] attribute with no arguments
                Ok(ModuleFieldAttribute::State {
                    codec_builder: None,
                })
            }
            syn::Meta::List(meta_list) => {
                if meta_list.tokens.is_empty() {
                    return Ok(ModuleFieldAttribute::State {
                        codec_builder: None,
                    });
                }

                let name_value = meta_list.parse_args::<syn::MetaNameValue>()?;

                if name_value.path.get_ident().map(Ident::to_string).as_deref()
                    != Some("codec_builder")
                {
                    return Err(syntax_err);
                }

                let codec_builder_path = match &name_value.value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit),
                        ..
                    }) => lit.parse_with(syn::Path::parse_mod_style)?,
                    _ => return Err(syntax_err),
                };

                Ok(ModuleFieldAttribute::State {
                    codec_builder: Some(codec_builder_path),
                })
            }
            _ => Err(syntax_err),
        }
    }

    fn parse_module_fields(data: &syn::Data) -> syn::Result<Vec<ModuleField>> {
        let data_struct = data_to_struct(data)?;
        let mut parsed_fields = vec![];

        for field in &data_struct.fields {
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

    fn check_zero_or_one_gas(fields: &[ModuleField]) -> syn::Result<()> {
        let gas_fields = fields
            .iter()
            .filter(|field| matches!(field.attr, ModuleFieldAttribute::Gas))
            .collect::<Vec<_>>();

        match gas_fields.len() {
            0 | 1 => Ok(()),
            _ => Err(syn::Error::new_spanned(
                gas_fields[1].ident.clone(),
                format!(
                    "The `gas` attribute is defined more than once, revisit field: {}",
                    gas_fields[1].ident,
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

    fn get_field_ident(field: &syn::Field) -> syn::Result<&Ident> {
        field.ident.as_ref().ok_or(syn::Error::new_spanned(
            field,
            "The `ModuleInfo` macro supports structs only, unnamed fields witnessed.",
        ))
    }

    fn get_field_attribute(field: &syn::Field) -> syn::Result<&Attribute> {
        let ident = get_field_ident(field)?;
        let mut attr = None;
        for a in &field.attrs {
            match a.path().segments[0].ident.to_string().as_str() {
                "state" | "module" | "id" | "kernel_module" | "phantom" => {
                    if attr.is_some() {
                        return Err(syn::Error::new_spanned(ident, "Only one attribute out of `#[kernel_module]`, `#[module]`, `#[state]`, `#[id]`,  and `#[phantom]` is allowed per field."));
                    }

                    attr = Some(a);
                }
                _ => {}
            }
        }

        if let Some(attr) = attr {
            Ok(attr)
        } else {
            Err(syn::Error::new_spanned(
                ident,
                format!("The field `{ident}` is missing an attribute: add `#[kernel_module]`, `#[module]`, `#[state]`, `#[id]`, or #[phantom]."),
            ))
        }
    }
}
