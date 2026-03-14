use soroban_sdk::{Address, String};

pub(crate) fn shorten(a: &Address) -> String {
    a.to_string().to_bytes().slice(52..56).to_string()
}
