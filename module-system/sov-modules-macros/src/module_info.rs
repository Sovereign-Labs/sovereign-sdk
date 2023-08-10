use proc_macro2::{self, Ident, Span};
use syn::{DataStruct, DeriveInput, ImplGenerics, PathArguments, TypeGenerics, WhereClause};

use crate::common::get_generics_type_param;

#[derive(Clone)]
struct StructNamedField {
    ident: proc_macro2::Ident,
    ty: syn::Type,
}

// A field can be either a state variable or another module.
// We don't generate prefix functions for imported modules as they are already generated.
#[derive(Clone)]
enum FieldKind {
    Address(StructNamedField),
    State(StructNamedField),
    Module(StructNamedField),
}

struct StructDef<'a> {
    ident: proc_macro2::Ident,
    impl_generics: ImplGenerics<'a>,
    type_generics: TypeGenerics<'a>,
    generic_param: &'a Ident,

    fields: Result<Vec<FieldKind>, syn::Error>,
    where_clause: Option<&'a WhereClause>,
}

pub(crate) fn derive_module_info(
    input: DeriveInput,
) -> Result<proc_macro::TokenStream, syn::Error> {
    let DeriveInput {
        data,
        ident,
        generics,
        ..
    } = input;

    let generic_param = get_generics_type_param(&generics, Span::call_site())?;

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let fields = get_fields_from_struct(&data);

    let struct_def = StructDef {
        ident,
        fields,
        impl_generics,
        type_generics,
        generic_param: &generic_param,
        where_clause,
    };

    let impl_prefix_functions = struct_def.impl_prefix_functions()?;
    let impl_new = struct_def.impl_module_info()?;

    Ok(quote::quote! {
        #impl_prefix_functions

        #impl_new
    }
    .into())
}

impl<'a> StructDef<'a> {
    // Creates a prefix function for each field of the underlying structure.
    fn impl_prefix_functions(&self) -> Result<proc_macro2::TokenStream, syn::Error> {
        let fields = self.fields.clone()?;

        let impl_prefix_functions = fields.iter().filter_map(|field| match field {
            FieldKind::State(field) => Some(make_prefix_func(field, &self.ident)),
            // Don't generate prefix functions for modules
            FieldKind::Module(_) => None,
            // Don't generate prefix functions for address
            FieldKind::Address(_) => None,
        });

        let impl_generics = &self.impl_generics;
        let ident = &self.ident;
        let ty_generics = &self.type_generics;
        let where_clause = self.where_clause;

        Ok(quote::quote! {
            impl #impl_generics #ident #ty_generics #where_clause{
                #(#impl_prefix_functions)*
            }
        })
    }

    // Implements the `ModuleInfo` trait.
    fn impl_module_info(&self) -> Result<proc_macro2::TokenStream, syn::Error> {
        let fields = self.fields.clone()?;
        let type_generics = &self.type_generics;

        let mut impl_self_init = Vec::default();
        let mut impl_self_body = Vec::default();
        let mut modules = Vec::default();

        let mut module_address = None;
        for field in fields.iter() {
            match field {
                FieldKind::State(field) => {
                    impl_self_init.push(make_init_state(field)?);
                    impl_self_body.push(&field.ident);
                }
                FieldKind::Module(field) => {
                    impl_self_init.push(make_init_module(field)?);
                    impl_self_body.push(&field.ident);
                    modules.push(&field.ident);
                }
                FieldKind::Address(field) => {
                    impl_self_init.push(make_init_address(
                        field,
                        &self.ident,
                        module_address,
                        self.generic_param,
                    )?);
                    impl_self_body.push(&field.ident);
                    module_address = Some(&field.ident);
                }
            };
        }

        let generic_param = self.generic_param;
        let impl_generics = &self.impl_generics;
        let ident = &self.ident;

        let where_clause = self.where_clause;

        let fn_address = make_fn_address(module_address)?;
        let fn_dependencies = make_fn_dependencies(modules);

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

                #fn_address

                #fn_dependencies
            }
        })
    }
}

// Extracts named fields form a struct or emits an error.
fn get_fields_from_struct(data: &syn::Data) -> Result<Vec<FieldKind>, syn::Error> {
    match data {
        syn::Data::Struct(data_struct) => get_fields_from_data_struct(data_struct),
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

fn get_fields_from_data_struct(data_struct: &DataStruct) -> Result<Vec<FieldKind>, syn::Error> {
    let mut output_fields = Vec::default();

    for original_field in data_struct.fields.iter() {
        let field_ident = original_field
            .ident
            .as_ref()
            .ok_or(syn::Error::new_spanned(
                &original_field.ident,
                "The `ModuleInfo` macro supports structs only, unnamed fields witnessed.",
            ))?;

        if original_field.attrs.is_empty() {
            return Err(syn::Error::new_spanned(
                &original_field.ident,
                "This field is missing an attribute: add `#[module]`, `#[state]` or `#[address]`. ",
            ));
        }

        for attribute in &original_field.attrs {
            let field = StructNamedField {
                ident: field_ident.clone(),
                ty: original_field.ty.clone(),
            };

            if attribute.path.segments[0].ident == "state" {
                output_fields.push(FieldKind::State(field));
            } else if attribute.path.segments[0].ident == "module" {
                output_fields.push(FieldKind::Module(field))
            } else if attribute.path.segments[0].ident == "address" {
                output_fields.push(FieldKind::Address(field))
            } else if attribute.path.segments[0].ident == "doc" {
                // Skip doc comments.
            } else {
                return Err(syn::Error::new_spanned(
                    field_ident,
                    "Only `#[module]`, `#[state]` or `#[address]` attributes are supported.",
                ));
            };
        }
    }
    Ok(output_fields)
}

fn prefix_func_ident(ident: &proc_macro2::Ident) -> proc_macro2::Ident {
    syn::Ident::new(&format!("_prefix_{ident}"), ident.span())
}

fn make_prefix_func(
    field: &StructNamedField,
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

fn make_fn_address(
    address_ident: Option<&proc_macro2::Ident>,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    match address_ident {
        Some(address_ident) => Ok(quote::quote! {
            fn address(&self) -> &<Self::Context as sov_modules_api::Spec>::Address {
               &self.#address_ident
            }
        }),
        None => Err(syn::Error::new(
            Span::call_site(),
            "The `ModuleInfo` macro requires `[address]` attribute.",
        )),
    }
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
fn make_init_state(field: &StructNamedField) -> Result<proc_macro2::TokenStream, syn::Error> {
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
        let #field_ident = #ty::new(state_prefix);
    })
}

fn make_init_module(field: &StructNamedField) -> Result<proc_macro2::TokenStream, syn::Error> {
    let field_ident = &field.ident;
    let ty = &field.ty;

    Ok(quote::quote! {
        let #field_ident = <#ty as ::std::default::Default>::default();
    })
}

fn make_init_address(
    field: &StructNamedField,
    struct_ident: &Ident,
    address: Option<&Ident>,
    generic_param: &Ident,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let field_ident = &field.ident;

    match address {
        Some(addr) => Err(syn::Error::new_spanned(
            addr,
            format!(
                "The `address` attribute is defined more than once, revisit field: {}",
                addr
            ),
        )),
        None => Ok(quote::quote! {
            use ::sov_modules_api::digest::Digest as _;
            let module_path = module_path!();
            let prefix = sov_modules_api::Prefix::new_module(module_path, stringify!(#struct_ident));
            let #field_ident : <#generic_param as sov_modules_api::Spec>::Address =
                <#generic_param as ::sov_modules_api::Spec>::Address::try_from(&prefix.hash::<#generic_param>())
                    .unwrap_or_else(|e| panic!("ModuleInfo macro error, unable to create an Address for module: {}", e));
        }),
    }
}
