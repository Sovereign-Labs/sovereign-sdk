extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::ItemFn;
use syn::FnArg;


pub fn wrap_function(input: ItemFn) -> Result<TokenStream, syn::Error> {
    // let name = &input.sig.ident;
    // let block = &input.block;
    // let signature = &input.sig;
    //
    // let result = quote! {
    //     #signature {
    //         // let before = env::get_cycle_count();
    //         let result = (|| #block)();
    //         // let after = env::get_cycle_count();
    //         // println!("{}: {}", stringify!(#name), after - before);
    //         println!("macro test");
    //         result
    //     }
    // };
    //
    // Ok(result.into())

        let visibility = &input.vis;
        let name = &input.sig.ident;
        let inputs = &input.sig.inputs;
        let output = &input.sig.output;
        let block = &input.block;

        if let Some(self_param) = inputs.first() {
            if matches!(self_param, FnArg::Receiver(_)) {
                // method
                let result = quote! {
                #visibility fn #name (#inputs) #output {
                    #[cfg(not(feature = "native"))]
                    let before = env::get_cycle_count();

                    let result = (|| #block)();

                    #[cfg(not(feature = "native"))]
                    let after = env::get_cycle_count();

                    #[cfg(not(feature = "native"))]
                    println!("{}: {}", stringify!(#name), after - before);

                    result
                }
            };
                println!("method: {}", result.to_string());
                Ok(result.into())
            } else {
                // function
                let result = quote! {
                #visibility fn #name (#inputs) #output {
                    let result = (|| #block)();
                    println!("function executed");
                    result
                }
            };
                Ok(result.into())
            }
        } else {
            // function without arguments
            let result = quote! {
            #visibility fn #name (#inputs) #output {
                let result = (|| #block)();
                println!("function executed");
                result
            }
        };
            Ok(result.into())
        }


}
