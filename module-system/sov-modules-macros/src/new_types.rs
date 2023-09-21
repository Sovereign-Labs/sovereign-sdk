use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, DeriveInput};

pub fn address_type_helper(input: DeriveInput) -> Result<TokenStream, syn::Error> {
    let name = &input.ident;
    let name_str = format!("{}", name);
    let attrs: Vec<Attribute> = input.attrs;

    let expanded = quote! {
        #[cfg(feature = "native")]
        #[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        #[schemars(bound = "C::Address: ::schemars::JsonSchema", rename = #name_str)]
        #[serde(transparent)]
        #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Clone, Debug, PartialEq, Eq, Hash)]
        #(#attrs)*
        pub struct #name<C: Context>(C::Address);

        #[cfg(not(feature = "native"))]
        #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Clone, Debug, PartialEq, Eq, Hash)]
        #(#attrs)*
        pub struct #name<C: Context>(C::Address);

        impl<C: Context> #name<C> {
            /// Public constructor
            pub fn new(address: &C::Address) -> Self {
                #name(address.clone())
            }

            /// Public getter
            pub fn get_address(&self) -> &C::Address {
                &self.0
            }
        }

        impl<C: Context> fmt::Display for #name<C>
        where
            C::Address: fmt::Display,
        {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl<C: Context> AsRef<[u8]> for #name<C>
        where
            C::Address: AsRef<[u8]>,
        {
            fn as_ref(&self) -> &[u8] {
                self.0.as_ref()
            }
        }
    };

    Ok(expanded.into())
}
