use super::common::{StructFieldExtractor, StructNamedField};
use proc_macro2::Ident;
use quote::format_ident;
use syn::DeriveInput;
use syn::GenericParam;
use syn::ImplGenerics;
use syn::TypeGenerics;
use syn::WhereClause;

struct StructDef<'a> {
    enum_ident: proc_macro2::Ident,
    impl_generics: ImplGenerics<'a>,
    type_generics: TypeGenerics<'a>,
    generic_param: &'a Ident,
    fields: Vec<StructNamedField>,
    where_clause: Option<&'a WhereClause>,
}

impl<'a> StructDef<'a> {
    fn new(
        ident: proc_macro2::Ident,
        fields: Vec<StructNamedField>,
        impl_generics: ImplGenerics<'a>,
        type_generics: TypeGenerics<'a>,
        generic_param: &'a Ident,
        where_clause: Option<&'a WhereClause>,
    ) -> Self {
        Self {
            enum_ident: format_ident!("{ident}Call"),
            fields,
            impl_generics,
            type_generics,
            generic_param,
            where_clause,
        }
    }

    /// Creates an enum type based on the underlying struct.
    fn create_enum(&self) -> proc_macro2::TokenStream {
        let enum_legs = self.fields.iter().map(|field| {
            let name = &field.ident;
            let ty = &field.ty;

            quote::quote!(
                #name(<#ty as sov_modules_api::Module>::CallMessage),
            )
        });

        let enum_ident = &self.enum_ident;
        let impl_generics = &self.impl_generics;
        let where_clause = &self.where_clause;

        quote::quote! {
            // This is generated code (won't be exposed to the users) and we allow non camel case for enum variants.
            #[allow(non_camel_case_types)]
            // TODO we should not hardcode the serialization format inside the macro:
            // https://github.com/Sovereign-Labs/sovereign/issues/97
            #[derive(borsh::BorshDeserialize, borsh::BorshSerialize)]
            enum #enum_ident #impl_generics #where_clause {
                #(#enum_legs)*
            }
        }
    }

    /// Implements `sov_modules_api::DispatchCall` for the enumeration created by `create_call_enum`.
    fn create_dispatch(&self) -> proc_macro2::TokenStream {
        let enum_ident = &self.enum_ident;
        let type_generics = &self.type_generics;

        let match_legs = self.fields.iter().map(|field| {
            let name = &field.ident;
            let ty = &field.ty;

            quote::quote!(
                #enum_ident::#name(message)=>{
                    let mut #name = <#ty as sov_modules_api::ModuleInfo::#type_generics>::new(storage.clone());
                    #name.call(message, context)
                },
            )
        });

        let impl_generics = &self.impl_generics;
        let where_clause = self.where_clause;
        let generic_param = self.generic_param;

        quote::quote! {
            impl #impl_generics sov_modules_api::DispatchCall for #enum_ident #type_generics #where_clause{
                type Context = #generic_param;

                fn dispatch(
                    self,
                    storage: <Self::Context as sov_modules_api::Spec>::Storage,
                    context: &Self::Context,
                ) -> core::result::Result<sov_modules_api::CallResponse, sov_modules_api::Error> {

                    match self{
                        #(#match_legs)*
                    }
                }
            }
        }
    }
}

pub(crate) struct DispatchCallMacro {
    field_extractor: StructFieldExtractor,
}

impl DispatchCallMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            field_extractor: StructFieldExtractor::new(name),
        }
    }

    pub(crate) fn derive_dispatch_call(
        &self,
        input: DeriveInput,
    ) -> Result<proc_macro::TokenStream, syn::Error> {
        let DeriveInput {
            data,
            ident,
            generics,
            ..
        } = input;

        let generic_param = match generics.params.first().unwrap() {
            GenericParam::Type(ty) => &ty.ident,
            GenericParam::Lifetime(lf) => {
                return Err(syn::Error::new_spanned(
                    lf,
                    "Lifetime parameters not supported.",
                ))
            }
            GenericParam::Const(cnst) => {
                return Err(syn::Error::new_spanned(
                    cnst,
                    "Const parameters not supported.",
                ))
            }
        };

        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let struct_def = StructDef::new(
            ident,
            fields,
            impl_generics,
            type_generics,
            generic_param,
            where_clause,
        );

        let call_enum = struct_def.create_enum();
        let create_dispatch_impl = struct_def.create_dispatch();

        Ok(quote::quote! {
            #call_enum

            #create_dispatch_impl
        }
        .into())
    }
}
