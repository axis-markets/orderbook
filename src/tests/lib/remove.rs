//! Order removal: `update` with a zero amount.
use super::setup::{
    actor, assert_no_custody, balance, fund, no_approvals, order_update, register_axis, removal,
    remove_orders, setup_test, store_order, trade, try_remove_orders,
};
use crate::events::OrderUpdatedEvent;
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{Event, Vec};

#[test]
fn test_remove_success() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 10000);

    let order_id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(balance(&e, &usd, &trader), 10000);

    // Remove the order: nothing to return, the funds never left the wallet
    let removed = remove_orders(&client, &trader, &[order_id]);
    let events = e.events().all().filter_by_contract(&contract_address);
    assert_eq!(removed, Vec::from_array(&e, [order_id]));
    let expected = OrderUpdatedEvent {
        id: order_id,
        price: PRECISION,
        amount: 0,
        expires: 0,
    };
    assert_eq!(events, [expected.to_xdr(&e, &contract_address)]);
    assert_eq!(balance(&e, &usd, &trader), 10000);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
    assert_eq!(client.order(&order_id), None);

    // Remove again - should do nothing (order already removed)
    assert!(remove_orders(&client, &trader, &[order_id]).is_empty());
}

#[test]
fn test_remove_non_existent_order() {
    let (e, trader, _, _, _) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    // Remove non-existent order - should not panic, just do nothing
    assert!(remove_orders(&client, &trader, &[999u128]).is_empty());
}

#[test]
fn test_remove_multiple_orders() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 100000);

    let order1 = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let order2 = store_order(&client, &trader, 2000, &usd, &eur, PRECISION);
    let order3 = store_order(&client, &trader, 3000, &usd, &eur, PRECISION);

    // Remove second order
    remove_orders(&client, &trader, &[order2]);
    assert_eq!(client.order(&order2), None);
    assert!(client.order(&order1).is_some());
    assert!(client.order(&order3).is_some());

    // Remove remaining orders in a single batched call
    remove_orders(&client, &trader, &[order1, order3]);
    assert_eq!(client.order(&order1), None);
    assert_eq!(client.order(&order3), None);
    assert_eq!(balance(&e, &usd, &trader), 100000);
}

#[test]
fn test_remove_after_partial_fill() {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);

    // Create a large order
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    // Partially fill it
    trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        300,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!(balance(&e, &usd, &maker), 9700);
    assert_eq!(client.order(&order_id).unwrap().amount, 700);

    // Now remove the remaining portion
    remove_orders(&client, &maker, &[order_id]);
    assert_eq!(client.order(&order_id), None);
    assert_eq!(balance(&e, &usd, &maker), 9700);
    assert_eq!(balance(&e, &eur, &maker), 300);
}

#[test]
fn test_remove_wrong_owner() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 10000);
    let order_id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    // Try to remove with different address - NotAuthorized
    let other_trader = actor(&e);
    assert_eq!(
        try_remove_orders(&client, &other_trader, &[order_id]),
        Some(701)
    );
    assert!(client.order(&order_id).is_some());
}

#[test]
fn test_remove_mixed_existent_and_non_existent() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 10000);

    let order1 = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let order2 = store_order(&client, &trader, 2000, &usd, &eur, PRECISION);

    // Mix real IDs with a non-existent one - the non-existent should be silently skipped
    let removed = remove_orders(&client, &trader, &[order1, 99999u128, order2]);
    assert_eq!(removed, Vec::from_array(&e, [order1, order2]));
    assert_eq!(client.order(&order1), None);
    assert_eq!(client.order(&order2), None);
}

#[test]
fn test_remove_ignores_price_and_expiration() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 10000);
    let order_id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    // an invalid price and a past expiration would fail a modification, not a removal
    let update = crate::trade::OrderUpdate {
        expires: 1,
        ..order_update(order_id, 0, 0)
    };
    let updates = Vec::from_array(&e, [update]);
    client.update(&trader, &updates, &no_approvals(&e));
    assert_eq!(client.order(&order_id), None);
}

#[test]
fn test_remove_and_modify_in_one_call() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 1000);
    let kept = store_order(&client, &trader, 500, &usd, &eur, PRECISION);
    let dropped = store_order(&client, &trader, 500, &usd, &eur, PRECISION);

    // a removal adds nothing to the backing the batch requires
    let updates = Vec::from_array(
        &e,
        [removal(dropped), order_update(kept, 1000, 2 * PRECISION)],
    );
    let updated = client.update(&trader, &updates, &no_approvals(&e));
    assert_eq!(updated, Vec::from_array(&e, [dropped, kept]));
    assert_eq!(client.order(&dropped), None);
    assert_eq!(client.order(&kept).unwrap().amount, 1000);
}
