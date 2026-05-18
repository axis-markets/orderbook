use super::setup::setup_test;
use crate::order::{OrderKind, TradeDirection};
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
    let (_, _, order_id) = client.trade(
        &TradeDirection::Sell,
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
    client.cancel(&Vec::from_array(&e, [order_id]), &trader);

    // Verify tokens were returned to trader
    assert_eq!(usd_client.balance(&trader), initial_balance);
    assert_eq!(usd_client.balance(&contract_address), 0);

    assert_eq!(client.order(&order_id), None);

    // Try to cancel again - should do nothing (order already removed)
    client.cancel(&Vec::from_array(&e, [order_id]), &trader);
    // Balance should remain the same
    assert_eq!(usd_client.balance(&trader), initial_balance);
}

#[test]
fn test_cancel_non_existent_order() {
    let (e, trader, _, _, _) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Try to cancel non-existent order - should not panic, just do nothing
    client.cancel(&Vec::from_array(&e, [999u64]), &trader);
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
    let (_, _, order1) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    let (_, _, order2) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &2000, &usd, &eur, &PRECISION, &Vec::new(&e));

    let (_, _, order3) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &3000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Cancel second order
    client.cancel(&Vec::from_array(&e, [order2]), &trader);

    // Verify 2000 tokens returned
    let expected_balance = initial_balance - 1000 - 3000;
    // Only orders 1 and 3 remain
    assert_eq!(usd_client.balance(&trader), expected_balance);

    // Cancel remaining orders in a single batched call
    client.cancel(&Vec::from_array(&e, [order1, order3]), &trader);

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
    let (_, _, order_id) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &maker, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Partially fill it
    client.trade(&TradeDirection::Sell, &OrderKind::Limit, &taker, &300, &eur, &usd, &PRECISION, &Vec::new(&e));

    let orders = Vec::from_array(&e, [order_id]);
    client.trade(&TradeDirection::Sell, &OrderKind::Fill, &taker, &300, &eur, &usd, &PRECISION, &orders);

    assert_eq!(usd_client.balance(&maker), 9000);
    // Now cancel the remaining portion
    client.cancel(&Vec::from_array(&e, [order_id]), &maker);

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
    let (_, _, order_id) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Try to cancel with different address - should panic
    let other_trader = Address::generate(&e);
    client.cancel(&Vec::from_array(&e, [order_id]), &other_trader);
}

#[test]
fn test_cancel_mixed_existent_and_non_existent() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &10000);

    let initial_balance = usd_client.balance(&trader);

    let (_, _, order1) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));
    let (_, _, order2) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &2000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Mix real IDs with a non-existent one - the non-existent should be silently skipped
    client.cancel(&Vec::from_array(&e, [order1, 99999u64, order2]), &trader);

    // Both real orders should have been cancelled and tokens returned
    assert_eq!(usd_client.balance(&trader), initial_balance);
    assert_eq!(usd_client.balance(&contract_address), 0);
    assert_eq!(client.order(&order1), None);
    assert_eq!(client.order(&order2), None);
}

#[test]
fn test_cancel_batched_same_asset() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &100000);

    let initial_balance = usd_client.balance(&trader);

    let (_, _, order1) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));
    let (_, _, order2) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &2000, &usd, &eur, &PRECISION, &Vec::new(&e));
    let (_, _, order3) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &3000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Cancel all three same-asset orders in one batched call.
    // The dispatcher should aggregate refunds into a single token.transfer.
    client.cancel(&Vec::from_array(&e, [order1, order2, order3]), &trader);

    assert_eq!(usd_client.balance(&trader), initial_balance);
    assert_eq!(usd_client.balance(&contract_address), 0);
    assert_eq!(client.order(&order1), None);
    assert_eq!(client.order(&order2), None);
    assert_eq!(client.order(&order3), None);
}
