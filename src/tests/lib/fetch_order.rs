use super::setup::{ensure_market, fund, no_orders, register_axis, setup_test, store_order};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::{Address, TryFromVal, Val, Vec};

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

#[test]
fn test_order_storage_encoding() {
    // Orders are stored as a positional vector [owner, selling, buying, amount, price] with the
    // expiration appended only when it is set; the view rebuilds the full order either way
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 1000000);
    ensure_market(&client, &usd, &eur);
    let expires = e.ledger().timestamp() + 3600;
    let create = |nonce: u64, expires: u64| {
        client
            .trade(
                &TradeDirection::Sell,
                &OrderKind::Limit,
                &trader,
                &1000,
                &usd,
                &eur,
                &(2 * PRECISION),
                &no_orders(&e),
                &nonce,
                &expires,
                &None,
            )
            .2
            .unwrap()
    };
    let lasting = create(1, 0);
    let expiring = create(2, expires);
    let stored = |id: u128| -> Vec<Val> {
        e.as_contract(&axis, || e.storage().persistent().get(&id).unwrap())
    };
    assert_eq!(stored(lasting).len(), 5);
    assert_eq!(stored(expiring).len(), 6);
    assert_eq!(
        Address::try_from_val(&e, &stored(lasting).get_unchecked(0)).unwrap(),
        trader
    );

    for (id, expected_expires) in [(lasting, 0), (expiring, expires)] {
        let order = client.order(&id).unwrap();
        assert_eq!(order.id, id);
        assert_eq!(order.owner, trader);
        assert_eq!(order.selling, usd);
        assert_eq!(order.buying, eur);
        assert_eq!(order.amount, 1000);
        assert_eq!(order.price, 2 * PRECISION);
        assert_eq!(order.expires, expected_expires);
    }
}
