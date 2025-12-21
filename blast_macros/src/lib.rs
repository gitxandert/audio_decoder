use proc_macro::{TokenStream, TokenTree, Ident, Span};

#[proc_macro]
pub fn var_args(var: TokenStream) -> TokenStream {
    let var = var.to_string().trim().to_string();
    let var_args = Ident::new(&format!("{}Args", var), Span::call_site());

    TokenStream::from(TokenTree::Ident(var_args))
}

macro_rules! commands {
    ( $( $var:ident ),* $(,)? ) => {
        #[derive(Copy, Clone, Debug)]
        pub enum Command {
            $(
                $var(var_args!($var)), // formats as {CmdType}Args
            )*
        }

        unsafe impl Send for Command {}
        unsafe impl Sync for Command {}
    }
}
