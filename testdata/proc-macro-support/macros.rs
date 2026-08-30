extern crate proc_macro;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn bundle_module(attribute: TokenStream, item: TokenStream) -> TokenStream {
    format!(
        "{item} mod dependency; pub const ATTRIBUTE_VALUE: usize = {attribute};"
    )
    .parse()
    .expect("generated module attribute should parse")
}

#[proc_macro_derive(Answer)]
pub fn answer(input: TokenStream) -> TokenStream {
    let name = input
        .into_iter()
        .skip_while(|token| token.to_string() != "struct")
        .nth(1)
        .expect("derive input should contain a struct name");
    let source_text = name.span().source_text();
    let name = name.to_string();
    assert_eq!(source_text.as_deref(), Some(name.as_str()));
    format!("impl {name} {{ fn answer() -> usize {{ 3 }} }}")
        .parse()
        .expect("generated derive should parse")
}

#[proc_macro]
pub fn include_value(_input: TokenStream) -> TokenStream {
    "include!(\"generated/value.rs\")"
        .parse()
        .expect("generated include should parse")
}
