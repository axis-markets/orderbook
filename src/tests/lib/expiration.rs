//! Order expiration: set by `trade` and `update`, enforced by every path that reads an order.
use super::setup::{
    actor, advance, balance, code, ensure_market, fund, nonce, register_axis, remove_orders,
    setup_test, store_order, trade, try_remove_orders, try_trade,
};
use crate::events::{OrderCreatedEvent, OrderUpdatedEvent};
use crate::order::{OrderKind, TradeDirection};
use crate::trade::TradeStep;
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{Address, Env, Event, Vec};

/// Order lifetime used throughout
const LIFETIME: u64 = 600;

/// Rest a sell order of `amount` at `price` that expires at `expires`, with an explicit nonce
#[allow(clippy::too_many_arguments)]
fn store_until(
    client: &AxisClient,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
    nonce: u64,
    expires: u64,
) -> u128 {
    let e = client.env.clone();
    fund(&e, selling, &client.address, trader, amount);
    ensure_market(client, selling, buying);
    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        trader,
        &amount,
        selling,
        buying,
        &price,
        &Vec::new(&e),
        &nonce,
        &expires,
        &None,
    );
    id.expect("the order must have been created")
}

fn deadline(e: &Env) -> u64 {
    e.ledger().timestamp() + LIFETIME
}

#[test]
fn test_limit_trade_stores_the_expiration() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let expires = deadline(&e);

    let id = store_until(&client, &trader, 1000, &usd, &eur, PRECISION, 1, expires);
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!(client.order(&id).unwrap().expires, expires);
    let expected = OrderCreatedEvent {
        selling: usd.clone(),
        buying: eur.clone(),
        id,
        owner: trader.clone(),
        price: PRECISION,
        amount: 1000,
        expires,
    };
    assert_eq!(events, [expected.to_xdr(&e, &axis)]);
}

#[test]
fn test_limit_trade_rejects_past_expiration() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 1000);
    let now = e.ledger().timestamp();

    for expires in [now, now - 1] {
        let res = client.try_trade(
            &TradeDirection::Sell,
            &OrderKind::Limit,
            &trader,
            &1000,
            &usd,
            &eur,
            &PRECISION,
            &Vec::new(&e),
            &nonce(),
            &expires,
            &None,
        );
        assert_eq!(code(res), Some(707));
    }
}

#[test]
fn test_fill_ignores_the_expiration() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 1000);
    fund(&e, &eur, &axis, &taker, 1000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    // no orders created from a Fill, so a stale timestamp does not matter
    let (sold, bought, created) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &taker,
        &400,
        &eur,
        &usd,
        &PRECISION,
        &Vec::from_array(&e, [id]),
        &nonce(),
        &1,
        &None,
    );
    assert_eq!((sold, bought, created), (400, 400, None));
}

#[test]
fn test_order_is_live_until_the_expiration() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &eur, &axis, &taker, 10000);
    let expires = deadline(&e);
    let id = store_until(&client, &maker, 1000, &usd, &eur, PRECISION, 1, expires);
    let orders = Vec::from_array(&e, [id]);

    // one second before the deadline the order still fills
    advance(&e, LIFETIME - 1);
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        100,
        &eur,
        &usd,
        PRECISION,
        &orders,
    );
    assert_eq!((sold, bought), (100, 100));

    // at the deadline it is gone: skipped by matching, hidden from the view
    advance(&e, 1);
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        100,
        &eur,
        &usd,
        PRECISION,
        &orders,
    );
    assert_eq!((sold, bought), (0, 0));
    assert!(client.order(&id).is_none());
    assert_eq!(balance(&e, &usd, &maker), 900);
}

#[test]
fn test_expired_order_fails_fill_or_kill() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &eur, &axis, &taker, 1000);
    let id = store_until(
        &client,
        &maker,
        1000,
        &usd,
        &eur,
        PRECISION,
        1,
        deadline(&e),
    );

    advance(&e, LIFETIME);
    let res = try_trade(
        &client,
        TradeDirection::Sell,
        OrderKind::FillOrKill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!(res, Some(709));
}

#[test]
fn test_limit_trade_created_over_an_expired_maker() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &eur, &axis, &taker, 1000);
    let expired = store_until(
        &client,
        &maker,
        1000,
        &usd,
        &eur,
        PRECISION,
        1,
        deadline(&e),
    );

    advance(&e, LIFETIME);
    let (sold, bought, created) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Limit,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [expired]),
    );
    assert_eq!((sold, bought), (0, 0));
    assert_eq!(client.order(&created.unwrap()).unwrap().amount, 1000);
}

#[test]
fn test_swap_does_not_route_through_an_expired_order() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let trader = actor(&e);
    fund(&e, &eur, &axis, &trader, 1000);
    let id = store_until(
        &client,
        &maker,
        1000,
        &usd,
        &eur,
        PRECISION,
        1,
        deadline(&e),
    );
    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: usd.clone(),
            orders: Vec::from_array(&e, [id]),
        }],
    );

    advance(&e, LIFETIME);
    let res = client.try_swap(
        &TradeDirection::Sell,
        &trader,
        &eur,
        &500,
        &500,
        &path,
        &None,
    );
    assert_eq!(code(res), Some(709));
    assert_eq!(balance(&e, &eur, &trader), 1000);
}

#[test]
fn test_crossfill_rejects_an_expired_taker_order() {
    let (e, owner, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    let cranker = actor(&e);
    let taker_order = store_until(
        &client,
        &owner,
        1000,
        &usd,
        &eur,
        PRECISION,
        1,
        deadline(&e),
    );
    let maker_order = store_until(&client, &maker, 1000, &eur, &usd, PRECISION, 2, 0);

    advance(&e, LIFETIME);
    let res = client.try_crossfill(&cranker, &taker_order, &Vec::from_array(&e, [maker_order]));
    assert_eq!(code(res), Some(710));
    assert_eq!(client.order(&maker_order).unwrap().amount, 1000);
}

#[test]
fn test_crossfill_skips_an_expired_maker_order() {
    let (e, owner, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    let cranker = actor(&e);
    let taker_order = store_until(&client, &owner, 1000, &usd, &eur, PRECISION, 1, 0);
    let expired = store_until(
        &client,
        &maker,
        1000,
        &eur,
        &usd,
        PRECISION,
        2,
        deadline(&e),
    );
    let live = store_until(&client, &maker, 400, &eur, &usd, PRECISION, 3, 0);

    advance(&e, LIFETIME);
    let (sold, bought, surplus) = client.crossfill(
        &cranker,
        &taker_order,
        &Vec::from_array(&e, [expired, live]),
    );
    assert_eq!((sold, bought, surplus), (400, 400, 0));
    assert_eq!(client.order(&taker_order).unwrap().amount, 600);
    assert!(client.order(&live).is_none());
}

#[test]
fn test_update_removes_an_expired_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let expires = deadline(&e);
    let id = store_until(&client, &trader, 1000, &usd, &eur, PRECISION, 1, expires);

    advance(&e, LIFETIME);
    assert_eq!(
        remove_orders(&client, &trader, &[id]),
        Vec::from_array(&e, [id])
    );
    let events = e.events().all().filter_by_contract(&axis);
    let removed = OrderUpdatedEvent {
        id,
        price: PRECISION,
        amount: 0,
        expires,
    };
    assert_eq!(events, [removed.to_xdr(&e, &axis)]);

    // removing someone else's expired order is still refused
    let other = store_until(
        &client,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
        2,
        deadline(&e),
    );
    advance(&e, LIFETIME);
    assert_eq!(try_remove_orders(&client, &actor(&e), &[other]), Some(701));
}

#[test]
fn test_expired_order_frees_its_nonce() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let id = store_until(
        &client,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
        5,
        deadline(&e),
    );

    // while live the nonce is taken
    fund(&e, &usd, &axis, &trader, 1000);
    let res = client.try_trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &trader,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
        &5,
        &0,
        &None,
    );
    assert_eq!(code(res), Some(711));

    // once expired the same nonce opens a fresh order under the same id
    advance(&e, LIFETIME);
    let again = store_until(&client, &trader, 700, &usd, &eur, 2 * PRECISION, 5, 0);
    assert_eq!(again, id);
    let order = client.order(&id).unwrap();
    assert_eq!(
        (order.amount, order.price, order.expires),
        (700, 2 * PRECISION, 0)
    );
}
