//! In-place order updates.
use super::setup::{
    actor, advance, approval, approve, balance, code, fund, no_approvals, order_update,
    register_axis, removal, remove_orders, set_price, setup_test, store_order, trade,
    try_update_one, update_one, ORACLE_DECIMALS,
};
use crate::events::OrderUpdatedEvent;
use crate::order::{OrderKind, TradeDirection};
use crate::trade::OrderUpdate;
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token, Event, Vec};

#[test]
fn test_update_amount_and_price() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    let updated = update_one(&client, &trader, order_update(id, 500, 2 * PRECISION));
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!(updated, Vec::from_array(&e, [id]));
    let order = client.order(&id).unwrap();
    assert_eq!(order.amount, 500);
    assert_eq!(order.price, 2 * PRECISION);
    assert_eq!(order.owner, trader);
    assert_eq!(order.selling, usd);
    // same id, one `mod` event, no transfer
    let expected = OrderUpdatedEvent {
        id,
        price: 2 * PRECISION,
        amount: 500,
        expires: 0,
    };
    assert_eq!(events, [expected.to_xdr(&e, &axis)]);
    assert_eq!(balance(&e, &usd, &trader), 10000);

    // growing the order is fine as long as it is backed
    update_one(&client, &trader, order_update(id, 8000, PRECISION));
    assert_eq!(client.order(&id).unwrap().amount, 8000);
}

#[test]
fn test_update_rejects_wrong_owner_and_skips_missing_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let other = actor(&e);

    assert_eq!(
        try_update_one(&client, &other, order_update(id, 500, PRECISION)),
        Some(701)
    );
    let updated = update_one(&client, &trader, order_update(999, 500, PRECISION));
    assert!(updated.is_empty());
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}

#[test]
fn test_update_validates_amount_price_and_size() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    assert_eq!(
        try_update_one(&client, &trader, order_update(id, -1, PRECISION)),
        Some(706)
    );
    assert_eq!(
        try_update_one(&client, &trader, order_update(id, 500, 0)),
        Some(705)
    );
    // 1 USD stroop at the lowest price is worth nothing
    assert_eq!(
        try_update_one(&client, &trader, order_update(id, 1, 1)),
        Some(720)
    );
    // below the minimum order value once USD is quoted at 1 USD per whole token
    set_price(&e, &usd, 10i128.pow(ORACLE_DECIMALS));
    client.requote(&usd, &eur);
    assert_eq!(
        try_update_one(&client, &trader, order_update(id, 500, PRECISION)),
        Some(720)
    );
}

#[test]
fn test_update_checks_backing() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    assert_eq!(
        try_update_one(&client, &trader, order_update(id, 20000, PRECISION)),
        Some(702)
    );
    approve(&e, &usd, &axis, &trader, 100);
    assert_eq!(
        try_update_one(&client, &trader, order_update(id, 2000, PRECISION)),
        Some(703)
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}

#[test]
fn test_update_with_approval_grants_the_allowance() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    approve(&e, &usd, &axis, &trader, 0);

    let updates = Vec::from_array(&e, [order_update(id, 3000, PRECISION)]);
    let approvals = Vec::from_array(&e, [approval(&e, &usd, 3000)]);
    client.update(&trader, &updates, &approvals);
    assert_eq!(client.order(&id).unwrap().amount, 3000);
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 3000);
}

#[test]
fn test_update_without_approval_keeps_the_allowance() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    approve(&e, &usd, &axis, &trader, 2000);

    // no approval for the asset: the standing allowance is left as it is
    update_one(&client, &trader, order_update(id, 1500, PRECISION));
    assert_eq!(client.order(&id).unwrap().amount, 1500);
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 2000);
}

#[test]
fn test_update_zero_approval_revokes_the_allowance() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    approve(&e, &usd, &axis, &trader, 2000);

    // an approval is granted as given: removing the last order and revoking the allowance
    let updates = Vec::from_array(&e, [removal(id)]);
    let approvals = Vec::from_array(&e, [approval(&e, &usd, 0)]);
    client.update(&trader, &updates, &approvals);
    assert!(client.order(&id).is_none());
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 0);

    // a zero allowance cannot back an order
    approve(&e, &usd, &axis, &trader, 2000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let updates = Vec::from_array(&e, [order_update(id, 500, PRECISION)]);
    let approvals = Vec::from_array(&e, [approval(&e, &usd, 0)]);
    assert_eq!(
        code(client.try_update(&trader, &updates, &approvals)),
        Some(703)
    );
}

#[test]
fn test_update_grants_approvals_without_updates() {
    let (e, trader, _, usd, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);

    let approvals = Vec::from_array(&e, [approval(&e, &usd, 1234)]);
    let updated = client.update(&trader, &Vec::new(&e), &approvals);
    assert!(updated.is_empty());
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 1234);
}

#[test]
fn test_update_skips_missing_ids() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let first = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let second = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let gone = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    remove_orders(&client, &trader, &[gone]);

    let updates = Vec::from_array(
        &e,
        [
            order_update(first, 500, PRECISION),
            order_update(gone, 500, PRECISION),
            order_update(second, 700, 3 * PRECISION),
        ],
    );
    let updated = client.update(&trader, &updates, &no_approvals(&e));
    assert_eq!(updated, Vec::from_array(&e, [first, second]));
    assert_eq!(client.order(&first).unwrap().amount, 500);
    assert_eq!(client.order(&second).unwrap().amount, 700);
    assert_eq!(client.order(&second).unwrap().price, 3 * PRECISION);
    assert!(client.order(&gone).is_none());
}

#[test]
fn test_update_sums_the_backing_per_asset() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 1000);
    let first = store_order(&client, &trader, 300, &usd, &eur, PRECISION);
    let second = store_order(&client, &trader, 300, &usd, &eur, PRECISION);

    let updates = Vec::from_array(
        &e,
        [
            order_update(first, 600, PRECISION),
            order_update(second, 600, PRECISION),
        ],
    );
    // each order alone is backed, the two together are not
    assert_eq!(
        code(client.try_update(&trader, &updates, &no_approvals(&e))),
        Some(702)
    );
    assert_eq!(client.order(&first).unwrap().amount, 300);
}

#[test]
fn test_update_approvals_cover_orders_selling_different_assets() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    fund(&e, &eur, &axis, &trader, 10000);
    let usd_order = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let eur_order = store_order(&client, &trader, 1000, &eur, &usd, PRECISION);
    approve(&e, &usd, &axis, &trader, 0);
    approve(&e, &eur, &axis, &trader, 0);

    let updates = Vec::from_array(
        &e,
        [
            order_update(usd_order, 500, PRECISION),
            order_update(eur_order, 700, PRECISION),
        ],
    );
    let approvals = Vec::from_array(&e, [approval(&e, &usd, 500), approval(&e, &eur, 700)]);
    let updated = client.update(&trader, &updates, &approvals);
    assert_eq!(updated.len(), 2);
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 500);
    assert_eq!(token::Client::new(&e, &eur).allowance(&trader, &axis), 700);
}

#[test]
fn test_update_last_approval_for_an_asset_wins() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let first = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let second = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    approve(&e, &usd, &axis, &trader, 0);

    let updates = Vec::from_array(
        &e,
        [
            order_update(first, 400, PRECISION),
            order_update(second, 400, PRECISION),
        ],
    );
    // the allowances are absolute: the second one replaces the first
    let approvals = Vec::from_array(&e, [approval(&e, &usd, 5000), approval(&e, &usd, 800)]);
    client.update(&trader, &updates, &approvals);
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 800);

    // an allowance below the batch total fails the backing check
    let approvals = Vec::from_array(&e, [approval(&e, &usd, 5000), approval(&e, &usd, 500)]);
    assert_eq!(
        code(client.try_update(&trader, &updates, &approvals)),
        Some(703)
    );
}

#[test]
fn test_update_sets_the_expiration() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let expires = e.ledger().timestamp() + 600;

    let update = OrderUpdate {
        expires,
        ..order_update(id, 800, PRECISION)
    };
    update_one(&client, &trader, update);
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!(client.order(&id).unwrap().expires, expires);
    let expected = OrderUpdatedEvent {
        id,
        price: PRECISION,
        amount: 800,
        expires,
    };
    assert_eq!(events, [expected.to_xdr(&e, &axis)]);

    // zero lifts the expiration again
    update_one(&client, &trader, order_update(id, 800, PRECISION));
    assert_eq!(client.order(&id).unwrap().expires, 0);
}

#[test]
fn test_update_rejects_past_expiration() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let now = e.ledger().timestamp();

    for expires in [now, now - 1] {
        let update = OrderUpdate {
            expires,
            ..order_update(id, 800, PRECISION)
        };
        assert_eq!(try_update_one(&client, &trader, update), Some(707));
    }
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}

#[test]
fn test_update_revives_expired_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    let expires = e.ledger().timestamp() + 60;
    update_one(
        &client,
        &trader,
        OrderUpdate {
            expires,
            ..order_update(id, 1000, PRECISION)
        },
    );

    advance(&e, 60);
    assert!(client.order(&id).is_none());
    // the new expiration must still be in the future
    let stale = OrderUpdate {
        expires,
        ..order_update(id, 500, PRECISION)
    };
    assert_eq!(try_update_one(&client, &trader, stale), Some(707));
    assert!(client.order(&id).is_none());

    // a future expiration brings the order back under the same id
    let update = OrderUpdate {
        expires: expires + 600,
        ..order_update(id, 500, PRECISION)
    };
    assert_eq!(
        update_one(&client, &trader, update),
        Vec::from_array(&e, [id])
    );
    let events = e.events().all().filter_by_contract(&axis);
    let order = client.order(&id).unwrap();
    assert_eq!((order.amount, order.expires), (500, expires + 600));
    let expected = OrderUpdatedEvent {
        id,
        price: PRECISION,
        amount: 500,
        expires: expires + 600,
    };
    assert_eq!(events, [expected.to_xdr(&e, &axis)]);
}

#[test]
fn test_update_revived_order_fills_again() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    let expires = e.ledger().timestamp() + 60;
    let expiring = OrderUpdate {
        expires,
        ..order_update(id, 1000, PRECISION)
    };
    update_one(&client, &maker, expiring);
    advance(&e, 60);

    // zero lifts the expiration: the order is live for good
    update_one(&client, &maker, order_update(id, 1000, PRECISION));
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        300,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (300, 300));
    assert_eq!(client.order(&id).unwrap().amount, 700);
}
