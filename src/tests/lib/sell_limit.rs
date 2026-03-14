use super::setup::{fake_asset, setup_test};
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};
use crate::order::OrderKind;

#[test]
fn test_sell_limit_creates_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint tokens to trader
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &10000);

    let initial_balance = usd_client.balance(&trader);
    // Create sell limit order
    let amount = 1000;
    // Create sell limit order
    let (sold, bought, order_id) =
        client.sell(&OrderKind::Limit, &trader, &amount, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Should not be filled (no matching orders)
    assert_eq!(sold, 0);
    assert_eq!(bought, 0);
    assert_eq!(order_id, 1);

    // Verify order was created
    let order = client.order(&order_id).unwrap();
    assert_eq!(order.amount, amount);
    assert_eq!(order.owner, trader);
    // Verify tokens were transferred to contract
    let trader_balance = usd_client.balance(&trader);
    let contract_balance = usd_client.balance(&contract_address);

    assert_eq!(trader_balance, initial_balance - amount);
    assert_eq!(contract_balance, amount);
}
//TODO: test partial and complete filling

#[test]
#[should_panic(expected = "#10")]
fn test_sell_limit_insufficient_balance() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Don't mint enough tokens
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &100);

    // Try to create order for more than balance - should panic
    client.sell(&OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));
}

#[test]
#[should_panic(expected = "\"contract call failed\", sell")]
fn test_sell_limit_requires_auth() {
    let e = Env::default();
    // Don't mock auth
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);

    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Don't mint enough tokens
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mock_all_auths().mint(&trader, &10000);

    // This should panic because trader auth is not provided
    client.sell(&OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));
}
