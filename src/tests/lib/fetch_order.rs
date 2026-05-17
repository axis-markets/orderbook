use super::setup::setup_test;
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::{token::StellarAssetClient, Vec};
use crate::order::{OrderKind, TradeDirection};

#[test]
fn test_order_retrieval() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint tokens to trader
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &1000000);

    let amount = 1000;
    let price = PRECISION;

    // Create an order
    let (_, _, order_id) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &amount, &usd, &eur, &price, &Vec::new(&e));

    // Retrieve the order
    let order = client.order(&order_id).unwrap();

    // Verify order details
    assert_eq!(order.owner, trader);
    assert_eq!(order.amount, amount);
    assert_eq!(order.selling, usd);
    assert_eq!(order.buying, eur);
    assert_eq!(order.price, price);
}
#[test]
fn test_last_after_order_creation() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint tokens to trader
    let usd_client = StellarAssetClient::new(&e, &usd);
    usd_client.mint(&trader, &1000000);

    // Initially, last order ID should be 0
    let last_id = client.last();
    assert_eq!(last_id, 0);
    // Create an order
    let (_, _, order_id) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &1000, &usd, &eur, &PRECISION, &Vec::new(&e));

    // Last order ID should match the created order
    let last_id = client.last();
    assert_eq!(last_id, order_id);
    assert_eq!(last_id, 1);

    // Create second order
    let (_, _, orderid2) = client.trade(&TradeDirection::Sell, &OrderKind::Limit, &trader, &2000, &usd, &eur, &PRECISION, &Vec::new(&e));
    assert_eq!(orderid2, 2);
    assert_eq!(client.last(), 2);
}

#[test]
fn test_order_not_found() {
    let (e, _, _, _, _) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Try to fetch non-existent order
    assert_eq!(client.order(&999), None);
    // Try to fetch order with ID 0 - should panic
    assert_eq!(client.order(&0), None);
}
