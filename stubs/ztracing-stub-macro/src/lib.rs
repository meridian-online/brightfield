//! Pass-through `#[instrument]` for the `ztracing` stub: returns the annotated
//! item unchanged, discarding the attribute arguments (`skip_all`, etc.).

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn instrument(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
