use proc_macro::{self};
use syn::{DataStruct, DeriveInput, ImplGenerics, PathArguments, TypeGenerics};

#[derive(Clone)]
struct StructNamedField {
    ident: proc_macro2::Ident,
    ty: syn::Type,
}

// A field can be either a state variable or another module.
// We don't generate prefix functions for imported modules as they are already generated.
#[derive(Clone)]
enum FieldKind {
    State(StructNamedField),
    Module(StructNamedField),
}

struct StructDef<'a> {
    ident: proc_macro2::Ident,
    impl_generics: ImplGenerics<'a>,
    type_generics: TypeGenerics<'a>,
    fields: Result<Vec<FieldKind>, syn::Error>,
}

pub(crate) fn module(input: DeriveInput) -> Result<proc_macro::TokenStream, syn::Error> {
    let DeriveInput {
        data,
        ident,
        generics,
        ..
    } = input;

    let (impl_generics, type_generics, _) = generics.split_for_impl();
    let fields = get_fields_from_struct(&data);

    let struct_def = StructDef {
        ident,
        fields,
        impl_generics,
        type_generics,
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
        });

        let impl_generics = &self.impl_generics;
        let ident = &self.ident;
        let ty_generics = &self.type_generics;

        Ok(quote::quote! {
            impl #impl_generics #ident #ty_generics {
                #(#impl_prefix_functions)*
            }
        })
    }

    // Implements the `ModuleInfo` trait.
    fn impl_module_info(&self) -> Result<proc_macro2::TokenStream, syn::Error> {
        let fields = self.fields.clone()?;

        let mut impl_self_init = Vec::default();
        let mut impl_self_body = Vec::default();

        for field in fields.iter() {
            match field {
                FieldKind::State(field) => {
                    impl_self_init.push(make_init_state(field)?);
                    impl_self_body.push(&field.ident);
                }
                FieldKind::Module(field) => {
                    impl_self_init.push(make_init_module(field, &self.type_generics)?);
                    impl_self_body.push(&field.ident);
                }
            };
        }

        let impl_generics = &self.impl_generics;
        let ident = &self.ident;
        let type_generics = &self.type_generics;

        Ok(quote::quote! {
            impl #impl_generics sov_modules_api::ModuleInfo #type_generics for #ident #type_generics {

                fn new(storage: #type_generics::Storage) -> Self {
                    #(#impl_self_init)*

                    Self{
                        #(#impl_self_body),*
                     }
                }
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
                "This field is missing an attribute: add `#[module]` or `#[state]`. ",
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
            } else if attribute.path.segments[0].ident == "doc" {
                // Skip doc comments.
            } else {
                return Err(syn::Error::new_spanned(
                    field_ident,
                    "Only `#[module]` or `#[state]` attributes are supported.",
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
    //      sov_modules_api::Prefix::new(module_path, module_name, field_ident)
    //   }
    quote::quote! {
        fn #prefix_func_ident() -> sov_modules_api::Prefix {
            let module_path = module_path!();
            sov_modules_api::Prefix::new(module_path, stringify!(#module_ident), stringify!(#field_ident))
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
    //  let field_ident = path::StateType::new(storage.clone(), state_prefix);
    Ok(quote::quote! {
        let state_prefix = Self::#prefix_fun().into();
        let #field_ident = #ty::new(storage.clone(), state_prefix);
    })
}

fn make_init_module<'a>(
    field: &StructNamedField,
    type_generics: &'a TypeGenerics<'a>,
) -> Result<proc_macro2::TokenStream, syn::Error> {
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

    Ok(quote::quote! {
        let #field_ident = <#ty #type_generics as sov_modules_api::ModuleInfo #type_generics>::new(storage.clone());
    })
}
