use super::setup::setup_test;
use crate::order::OrderKind;
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Vec};

#[test]
fn test_cancel_success() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint tokens to trader
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &10000);

    let initial_balance = usd_client.balance(&trader);
    let amount = 1000;

    // Create an order
    let (_, _, order_id) = client.sell(
        &OrderKind::Limit,
        &trader,
        &amount,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );

    // Verify tokens were transferred to contract
    assert_eq!(usd_client.balance(&trader), initial_balance - amount);
    assert_eq!(usd_client.balance(&contract_address), amount);

    // Cancel the order
    client.cancel(&order_id, &trader);

    // Verify tokens were returned to trader
    assert_eq!(usd_client.balance(&trader), initial_balance);
    assert_eq!(usd_client.balance(&contract_address), 0);

    assert_eq!(client.order(&order_id), None);

    // Try to cancel again - should do nothing (order already removed)
    client.cancel(&order_id, &trader);
    // Balance should remain the same
    assert_eq!(usd_client.balance(&trader), initial_balance);
}

#[test]
fn test_cancel_non_existent_order() {
    let (e, trader, _, _, _) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Try to cancel non-existent order - should not panic, just do nothing
    client.cancel(&999, &trader);
}

#[test]
fn test_cancel_multiple_orders() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint tokens to trader
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &100000);

    let initial_balance = usd_client.balance(&trader);

    // Create multiple orders
    let (_, _, order1) = client.sell(&OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    let (_, _, order2) = client.sell(&OrderKind::Limit, &trader, &2000, &usd, &eur, &PRECISION, &Vec::new(&e));

    let (_, _, order3) = client.sell(&OrderKind::Limit, &trader, &3000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Cancel second order
    client.cancel(&order2, &trader);

    // Verify 2000 tokens returned
    let expected_balance = initial_balance - 1000 - 3000;
    // Only orders 1 and 3 remain
    assert_eq!(usd_client.balance(&trader), expected_balance);

    // Cancel remaining orders
    client.cancel(&order1, &trader);
    client.cancel(&order3, &trader);

    // Verify all tokens returned
    assert_eq!(usd_client.balance(&trader), initial_balance);
}

#[test]
fn test_cancel_after_partial_fill() {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let taker = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);

    // Mint tokens
    usd_client.mint(&maker, &10000);
    eur_client.mint(&taker, &10000);

    // Create a large order
    let (_, _, order_id) = client.sell(&OrderKind::Limit, &maker, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Partially fill it
    client.sell(&OrderKind::Limit, &taker, &300, &eur, &usd, &PRECISION, &Vec::new(&e));

    let orders = Vec::from_array(&e, [order_id]);
    client.sell(&OrderKind::Fill, &taker, &300, &eur, &usd, &PRECISION, &orders);

    assert_eq!(usd_client.balance(&maker), 9000);
    // Now cancel the remaining portion
    client.cancel(&order_id, &maker);

    // The cancel should return the remaining amount
    assert_eq!(usd_client.balance(&maker), 9700);
    assert_eq!(eur_client.balance(&maker), 300);
}

#[test]
#[should_panic]
fn test_cancel_wrong_owner() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint tokens to trader
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &10000);

    // Create an order
    let (_, _, order_id) = client.sell(&OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Try to cancel with different address - should panic
    let other_trader = Address::generate(&e);
    client.cancel(&order_id, &other_trader);
}
