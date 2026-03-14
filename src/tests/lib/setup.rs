use soroban_sdk::testutils::Address as _;
use soroban_sdk::{log, Address, Env};
use crate::utils::shorten;

/// Create a fake Stellar asset for testing
pub fn fake_asset(env: &Env, issuer: &Address) -> Address {
    env.register_stellar_asset_contract_v2(issuer.clone())
        .address()
}

/// Setup a basic test environment with admin, trader, issuer, and two assets (USD and EUR)
/// Returns: (Env, admin, trader, issuer, usd, eur)
pub fn setup_test() -> (Env, Address, Address, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    log!(&e, "setup | USD: {}, EUR: {}", shorten(&usd), shorten(&eur));
    (e, trader, issuer, usd, eur)
}
