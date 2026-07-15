use proc_macro2::{Ident, Span, TokenStream};
use std::{fs, path::PathBuf};
use syn::punctuated::Punctuated;

#[derive(Clone)]
pub struct StaticBytesInput {
    pub name: Ident,
    pub file: syn::LitStr,
}


impl syn::parse::Parse for StaticBytesInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;
	let _: syn::Token![,] = input.parse()?;
        let file = input.parse()?;

        Ok(Self {
	    name,
	    file,
        })
    }
}


pub fn make_static_bytes(input: &StaticBytesInput) -> syn::Result<TokenStream> {
    let mut path = PathBuf::new();
    path.push(env!("CONSTANTS_MANIFEST_PATH"));
    path.pop();
    path.push(input.file.value());
    
    let content = fs::read_to_string(&path).map_err(|e| {
	syn::Error::new(
            input.file.span(),
            format!("failed to read `{}`: {}", path.display(), e),
        )})?;

    let len = content.len();
    let elems = content.as_bytes().iter().map(|b| {
        let lit = syn::LitInt::new(&format!("{b}u8"), Span::call_site());
        syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Int(lit),
        })
    });
    let content = syn::Expr::Array(syn::ExprArray {
        attrs: Vec::new(),
        bracket_token: syn::token::Bracket::default(),
        elems: Punctuated::from_iter(elems),
    });

    let name = quote::format_ident!("{}", input.name);

    // MacOS has a different convention for section names.
    #[cfg(target_os = "macos")]
    let link_section = format!("__DATA,__{}", name);
    #[cfg(target_os = "linux")]
    let link_section = format!(".data.{}", name);

    
    Ok(quote::quote!(
	#[unsafe(link_section = #link_section)]
	static #name : [u8; #len] = #content;
    ))    
}
