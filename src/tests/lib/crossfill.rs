use super::setup::{
    actor, approve, assert_no_custody, balance, code, fund, no_orders, register_axis, setup_test,
    store_order,
};
use crate::events::{OrderUpdatedEvent, TradeEvent};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{Address, Env, Event, Vec};

#[test]
fn test_crossfill_empty_orders_list() {
    let (e, trader, _issuer, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let arbitrageur = actor(&e);
    fund(&e, &usd, &contract_address, &trader, 10000);
    let taker_order_id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    // Try to fill with empty orders list
    let (sold, bought, profit) = client.crossfill(&arbitrageur, &taker_order_id, &no_orders(&e));
    assert_eq!((sold, bought, profit), (0, 0, 0));

    // Taker order should still exist with original amount
    let taker_order = client.order(&taker_order_id).unwrap();
    assert_eq!(taker_order.amount, 1000);
}

#[test]
fn test_crossfill_taker_not_found() {
    let (e, trader, _issuer, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let maker = actor(&e);
    fund(&e, &eur, &contract_address, &maker, 10000);
    let maker_order_id = store_order(&client, &maker, 1000, &eur, &usd, PRECISION);

    // Try to fill non-existent taker order
    let orders = Vec::from_array(&e, [maker_order_id]);
    assert_eq!(
        code(client.try_crossfill(&trader, &999, &orders)),
        Some(710)
    );
}

/// Taker sells 1000 USD for EUR at `taker_price`, maker sells `maker_amount` EUR for USD at
/// `maker_price`. Returns (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id).
fn setup_cross<'a>(
    e: &'a Env,
    usd: &Address,
    eur: &Address,
    taker_price: i128,
    maker_amount: i128,
    maker_price: i128,
) -> (AxisClient<'a>, Address, Address, Address, u128, u128) {
    let contract_address = register_axis(e);
    let client = AxisClient::new(e, &contract_address);

    let taker_owner = actor(e);
    let maker = actor(e);
    let arbitrageur = actor(e);

    fund(e, usd, &contract_address, &taker_owner, 1000);
    fund(e, eur, &contract_address, &maker, maker_amount);

    let taker_order_id = store_order(&client, &taker_owner, 1000, usd, eur, taker_price);
    let maker_order_id = store_order(&client, &maker, maker_amount, eur, usd, maker_price);
    (
        client,
        arbitrageur,
        taker_owner,
        maker,
        taker_order_id,
        maker_order_id,
    )
}

#[test]
fn test_crossfill_partially_fills_taker() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, PRECISION, 400, PRECISION);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought, profit) = client.crossfill(&arbitrageur, &taker_order_id, &orders);
    assert_eq!((sold, bought, profit), (400, 400, 0));

    // taker order must be decremented by the sold amount
    let taker_order = client.order(&taker_order_id).unwrap();
    assert_eq!(taker_order.amount, 600);
    // maker order executed in full and removed
    assert!(client.order(&maker_order_id).is_none());

    // arbitrageur nets to zero, counterparties receive their tokens from each other
    assert_eq!(balance(&e, &usd, &arbitrageur), 0);
    assert_eq!(balance(&e, &eur, &arbitrageur), 0);
    assert_eq!(balance(&e, &eur, &taker_owner), 400);
    assert_eq!(balance(&e, &usd, &taker_owner), 600);
    assert_eq!(balance(&e, &usd, &maker), 400);
    assert_no_custody(&e, &client.address, &[&usd, &eur]);
}

#[test]
fn test_crossfill_fully_fills_taker() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, PRECISION, 1500, PRECISION);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought, profit) = client.crossfill(&arbitrageur, &taker_order_id, &orders);
    assert_eq!((sold, bought, profit), (1000, 1000, 0));

    // taker order executed in full and removed
    assert!(client.order(&taker_order_id).is_none());
    // maker keeps the remainder
    let maker_order = client.order(&maker_order_id).unwrap();
    assert_eq!(maker_order.amount, 500);

    assert_eq!(balance(&e, &eur, &taker_owner), 1000);
    assert_eq!(balance(&e, &usd, &maker), 1000);
    assert_no_custody(&e, &client.address, &[&usd, &eur]);
}

#[test]
fn test_crossfill_pays_the_spread_to_the_trader() {
    // Taker order: 1000 USD for EUR at 1.0 (owner wants 1 EUR per USD).
    // Maker order: 1200 EUR for USD at 0.8 (0.8 USD per EUR, so 1.25 EUR per USD).
    // The owner spends 960 USD for the maker's 1200 EUR, is paid exactly 960 EUR, and the
    // 240 EUR crossed spread goes to the trader.
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, PRECISION, 1200, 8 * PRECISION / 10);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought, profit) = client.crossfill(&arbitrageur, &taker_order_id, &orders);
    let events = e.events().all().filter_by_contract(&client.address);
    assert_eq!((sold, bought, profit), (960, 1200, 240));

    assert_eq!(balance(&e, &usd, &taker_owner), 40);
    assert_eq!(balance(&e, &eur, &taker_owner), 960);
    assert_eq!(balance(&e, &usd, &maker), 960);
    assert_eq!(balance(&e, &eur, &maker), 0);
    assert_eq!(balance(&e, &eur, &arbitrageur), 240);
    assert_no_custody(&e, &client.address, &[&usd, &eur]);
    assert_eq!(client.order(&taker_order_id).unwrap().amount, 40);
    assert!(client.order(&maker_order_id).is_none());

    // two trades: the maker fill (owner as taker) and the taker order's own fill
    let maker_fill = TradeEvent {
        selling: usd.clone(),
        buying: eur.clone(),
        order: maker_order_id,
        taker: taker_owner.clone(),
        maker: maker.clone(),
        sold: 960,
        bought: 1200,
        left: 0,
    };
    let taker_fill = TradeEvent {
        selling: eur.clone(),
        buying: usd.clone(),
        order: taker_order_id,
        taker: arbitrageur.clone(),
        maker: taker_owner.clone(),
        sold: 960,
        bought: 960,
        left: 40,
    };
    assert_eq!(
        events,
        [
            maker_fill.to_xdr(&e, &client.address),
            taker_fill.to_xdr(&e, &client.address)
        ]
    );
}

#[test]
fn test_crossfill_skips_fills_that_would_underpay_the_owner() {
    // Taker order sells 1 USD wanting 2.5 EUR per USD; maker sells 3 EUR at 0.4 USD per EUR.
    // 1 USD buys floor(2.5) = 2 EUR for ceil(0.8) = 1 USD, but the owner is owed
    // ceil(1 * 2.5) = 3 EUR: the rounding would leave the trader short, so the fill is skipped
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    let taker_owner = actor(&e);
    let maker = actor(&e);
    let arbitrageur = actor(&e);
    fund(&e, &usd, &contract_address, &taker_owner, 1);
    fund(&e, &eur, &contract_address, &maker, 3);
    let taker_order_id = store_order(&client, &taker_owner, 1, &usd, &eur, 25 * PRECISION / 10);
    let maker_order_id = store_order(&client, &maker, 3, &eur, &usd, 4 * PRECISION / 10);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought, profit) = client.crossfill(&arbitrageur, &taker_order_id, &orders);
    assert_eq!((sold, bought, profit), (0, 0, 0));
    assert!(client.order(&taker_order_id).is_some());
    assert!(client.order(&maker_order_id).is_some());
}

#[test]
fn test_crossfill_removes_unbacked_taker_order() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, taker_owner, _maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, PRECISION, 400, PRECISION);
    // the owner revokes the allowance: the taker order is no longer backed
    approve(&e, &usd, &client.address, &taker_owner, 0);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought, profit) = client.crossfill(&arbitrageur, &taker_order_id, &orders);
    let events = e.events().all().filter_by_contract(&client.address);
    assert_eq!((sold, bought, profit), (0, 0, 0));
    assert!(client.order(&taker_order_id).is_none());
    assert_eq!(client.order(&maker_order_id).unwrap().amount, 400);
    let removed = OrderUpdatedEvent {
        id: taker_order_id,
        price: PRECISION,
        amount: 0,
        expires: 0,
    };
    assert_eq!(events, [removed.to_xdr(&e, &client.address)]);
}

#[test]
fn test_crossfill_requires_trader_auth() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, _taker_owner, _maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, PRECISION, 400, PRECISION);

    // drop the blanket auth mock: nobody authorizes the call now
    e.set_auths(&[]);
    let orders = Vec::from_array(&e, [maker_order_id]);
    let res = client.try_crossfill(&arbitrageur, &taker_order_id, &orders);
    assert!(
        res.is_err(),
        "crossfill must fail without trader authorization"
    );

    // nothing changed on the book
    assert_eq!(client.order(&taker_order_id).unwrap().amount, 1000);
    assert_eq!(client.order(&maker_order_id).unwrap().amount, 400);
}
