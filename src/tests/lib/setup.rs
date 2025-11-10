use crate::SorobanOrderbook;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Create a fake Stellar asset for testing
pub fn fake_asset(env: &Env, issuer: &Address) -> Address {
    env.register_stellar_asset_contract_v2(issuer.clone())
        .address()
}

/// Setup a basic test environment with admin, trader, issuer, and two assets (USD and EUR)
/// Returns: (Env, admin, trader, issuer, usd, eur)
pub fn setup_test() -> (Env, Address, Address, Address, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    (e, admin, trader, issuer, usd, eur)
}

/// Setup a minimal test environment for configuration tests
/// Returns: (Env, admin, contract_address)
pub fn setup_test_minimal() -> (Env, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let contract_address = e.register(SorobanOrderbook, ());
    (e, admin, contract_address)
}
