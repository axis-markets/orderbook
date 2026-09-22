use super::setup::{fund, register_axis, setup_test, store_order};
use crate::{orderbook::PRECISION, AxisClient};

#[test]
fn test_order_retrieval() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 1000000);

    let amount = 1000;
    let price = PRECISION;
    let order_id = store_order(&client, &trader, amount, &usd, &eur, price);

    // Retrieve the order
    let order = client.order(&order_id).unwrap();

    // Verify order details
    assert_eq!(order.id, order_id);
    assert_eq!(order.owner, trader);
    assert_eq!(order.amount, amount);
    assert_eq!(order.selling, usd);
    assert_eq!(order.buying, eur);
    assert_eq!(order.price, price);
    assert_eq!(order.expires, 0);
}

#[test]
fn test_order_not_found() {
    let (e, _, _, _, _) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    // Try to fetch non-existent orders
    assert_eq!(client.order(&999), None);
    assert_eq!(client.order(&0), None);
    assert_eq!(client.order(&u128::MAX), None);
}
