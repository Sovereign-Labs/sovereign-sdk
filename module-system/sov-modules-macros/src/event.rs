use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub fn derive_event(input: DeriveInput) -> Result<proc_macro::TokenStream, syn::Error> {
    match &input.data {
        Data::Enum(data_enum) => {
            let enum_name = &input.ident;
            let event_keys = data_enum.variants.iter().map(|v| {
                let variant_name = &v.ident;
                let variant_str = variant_name.to_string();

                match &v.fields {
                    Fields::Unit => {
                        quote! {
                            #enum_name::#variant_name => #variant_str,
                        }
                    }
                    Fields::Unnamed(_) => {
                        quote! {
                            #enum_name::#variant_name(_) => #variant_str,
                        }
                    }
                    Fields::Named(_) => {
                        quote! {
                            #enum_name::#variant_name { .. } => #variant_str,
                        }
                    }
                }
            });
            let gen = quote! {
                impl ::sov_modules_api::Event for #enum_name {
                    fn event_key(&self) -> &'static str {
                        match self {
                            #(#event_keys)*
                        }
                    }
                }
            };
            Ok(gen.into())
        }
        Data::Struct(st) => Err(syn::Error::new_spanned(
            st.struct_token,
            "The Event macro supports enums only.",
        )),
        Data::Union(un) => Err(syn::Error::new_spanned(
            un.union_token,
            "The Event macro supports enums only.",
        )),
    }
}
