use proc_macro::TokenStream;
use quote::quote;
use syn::ItemFn;

pub fn offchain_generator(function: ItemFn) -> Result<TokenStream, syn::Error> {
    let visibility = &function.vis;
    let name = &function.sig.ident;
    let inputs = &function.sig.inputs;
    let output = &function.sig.output;
    let block = &function.block;
    let generics = &function.sig.generics;
    let where_clause = &function.sig.generics.where_clause;
    let asyncness = &function.sig.asyncness;

    let output = quote! {
        // The "real" function
        #[cfg(feature = "offchain")]
        #visibility #asyncness fn #name #generics(#inputs) #output #where_clause {
            #block
        }

        // The no-op function
        #[cfg(not(feature = "offchain"))]
        #[allow(unused_variables)]
        #visibility #asyncness fn #name #generics(#inputs) #output #where_clause {
            // Do nothing. Should be optimized away
        }
    };

    Ok(output.into())
}
