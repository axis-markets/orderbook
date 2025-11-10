use super::setup::setup_test_minimal as setup_test;
use crate::{SorobanOrderbook, SorobanOrderbookClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn test_configure_success() {
    let (e, admin, contract_address) = setup_test();
    let client = SorobanOrderbookClient::new(&e, &contract_address);

    // Configure with fee of 5‰ (0.5%)
    client.configure(&admin, &5);

    let new_admin = Address::generate(&e);
    // Configure with zero fee
    client.configure(&new_admin, &0);
}

#[test]
#[should_panic]
fn test_configure_requires_auth() {
    let e = Env::default();
    // Don't mock auths - this should fail
    let admin = Address::generate(&e);
    let contract_address = e.register(SorobanOrderbook, ());
    let client = SorobanOrderbookClient::new(&e, &contract_address);

    // This should panic because auth is not provided
    client.configure(&admin, &5);
}
