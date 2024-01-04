use proc_macro::TokenStream;
use proc_macro2::{Ident, TokenTree};
use quote::{quote, ToTokens};
use regex::Regex;
use syn::{
	parse2, parse_macro_input, parse_str, punctuated::Punctuated, token::Comma, Expr, FnArg, ItemFn, ReturnType, Type
};

#[proc_macro_attribute]
pub fn command(_: TokenStream, item: TokenStream) -> TokenStream {
	let function = parse_macro_input!(item as ItemFn);

	let orig_name = &function.sig.ident;
	let name = Ident::new(&format!("rs_{}", orig_name), orig_name.span());

	let mut args: Punctuated<FnArg, Comma> = Punctuated::new();
	for arg in function.sig.inputs.iter() {
		if let FnArg::Typed(arg) = arg.to_owned() {
			args.push(FnArg::Typed(syn::PatType {
				attrs: arg.attrs,
				pat: arg.pat,
				colon_token: arg.colon_token,
				ty: Box::new(
					parse2(
						arg.ty
							.to_token_stream()
							.into_iter()
							.filter(
								|token: &TokenTree| !matches!(token, TokenTree::Punct(x) if x.to_string() == "&" || x.to_string() == "mut")
							)
							.collect::<proc_macro2::TokenStream>()
					)
					.unwrap()
				)
			}));
		}
	}

	let return_type = match &function.sig.output {
		ReturnType::Type(arrow, ty) => {
			let ok: Type = parse_str(
				&Regex::new("Result<(.*)>")
					.unwrap()
					.captures(&ty.to_token_stream().to_string().replace(' ', ""))
					.unwrap()
					.get(1)
					.unwrap()
					.as_str()
			)
			.unwrap();

			ReturnType::Type(arrow.to_owned(), Box::new(parse2(quote!(Result<#ok, String>)).unwrap()))
		}

		_ => panic!("Return type invalid")
	};

	let mut call: Punctuated<Expr, Comma> = Punctuated::new();
	call.extend(function.sig.inputs.iter().map(|x| {
		match x {
			FnArg::Typed(arg) => parse2::<Expr>(
				arg.ty
					.to_token_stream()
					.into_iter()
					.filter(
						|token: &TokenTree| matches!(token, TokenTree::Punct(x) if x.to_string() == "&"|| x.to_string() == "mut")
					)
					.chain(arg.pat.to_token_stream().into_iter())
					.collect()
			)
			.unwrap(),

			_ => panic!("Input arg invalid")
		}
	}));

	quote! {
		#[tauri::command]
		#[specta::specta]
		pub fn #name(#args) #return_type {
			#orig_name(#call).map_err(|e| format!("{e:?}"))
		}

		#function
	}
	.into()
}
