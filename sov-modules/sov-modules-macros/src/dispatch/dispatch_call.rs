use super::common::parse_generic_params;
use super::common::StructDef;
use super::common::StructFieldExtractor;
use syn::DeriveInput;

impl<'a> StructDef<'a> {
    fn create_call_enum_legs(&self) -> Vec<proc_macro2::TokenStream> {
        self.fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                let ty = &field.ty;

                quote::quote!(
                    #name(<#ty as sov_modules_api::Module>::CallMessage),
                )
            })
            .collect()
    }

    /// Implements `sov_modules_api::DispatchCall` for the enumeration created by `create_enum`.
    fn create_call_dispatch(&self) -> proc_macro2::TokenStream {
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

                fn dispatch_call(
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
    name: &'static str,
    field_extractor: StructFieldExtractor,
}

impl DispatchCallMacro {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
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

        let generic_param = parse_generic_params(&generics)?;

        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
        let fields = self.field_extractor.get_fields_from_struct(&data)?;

        let struct_def = StructDef::new(
            ident,
            fields,
            impl_generics,
            type_generics,
            &generic_param,
            where_clause,
            self.name,
        );

        let call_enum_legs = struct_def.create_call_enum_legs();
        let call_enum = struct_def.create_enum(&call_enum_legs);
        let create_dispatch_impl = struct_def.create_call_dispatch();

        Ok(quote::quote! {
            #call_enum

            #create_dispatch_impl
        }
        .into())
    }
}
