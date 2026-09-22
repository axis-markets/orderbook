//! Orders are backed by the maker's balance and allowance instead of a deposit: a fill is
//! trimmed to what the maker can deliver, and orders nobody can fill are removed.
use super::setup::{
    actor, advance_ledgers, approve, assert_no_custody, balance, fund, open_market, register_axis,
    setup_test, store_order, trade, try_trade,
};
use crate::events::{OrderUpdatedEvent, TradeEvent};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token, Event, Vec};

#[test]
fn test_fill_trimmed_to_allowance() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    // the maker lowers the allowance below the order
    approve(&e, &usd, &axis, &maker, 400);

    let (sold, bought, created) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!((sold, bought, created), (400, 400, None));
    // the backing left is zero, so the order is removed rather than kept at 600
    assert!(client.order(&id).is_none());
    assert_eq!(balance(&e, &usd, &maker), 9600);
    assert_eq!(balance(&e, &eur, &maker), 400);
    assert_eq!(balance(&e, &usd, &taker), 400);
    let expected = TradeEvent {
        selling: eur.clone(),
        buying: usd.clone(),
        order: id,
        taker: taker.clone(),
        maker: maker.clone(),
        sold: 400,
        bought: 400,
        left: 0,
    };
    assert_eq!(events, [expected.to_xdr(&e, &axis)]);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

#[test]
fn test_fill_trimmed_to_balance() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 1000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, 2 * PRECISION);
    // the maker moves most of the balance away
    token::Client::new(&e, &usd).transfer(&maker, &taker, &700);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        2000,
        &eur,
        &usd,
        PRECISION / 2,
        &Vec::from_array(&e, [id]),
    );
    // 300 USD left at 2 EUR/USD cost 600 EUR
    assert_eq!((sold, bought), (600, 300));
    assert!(client.order(&id).is_none());
    assert_eq!(balance(&e, &usd, &maker), 0);
    assert_eq!(balance(&e, &eur, &maker), 600);
}

#[test]
fn test_unbacked_order_is_removed() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    approve(&e, &usd, &axis, &maker, 0);

    let (sold, bought, created) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!((sold, bought, created), (0, 0, None));
    assert!(client.order(&id).is_none());
    let removed = OrderUpdatedEvent {
        id,
        price: PRECISION,
        amount: 0,
        expires: 0,
    };
    assert_eq!(events, [removed.to_xdr(&e, &axis)]);
    assert_eq!(balance(&e, &eur, &taker), 10000);
}

#[test]
fn test_unbacked_order_fails_fill_or_kill_without_trace() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    approve(&e, &usd, &axis, &maker, 0);

    // the plan fills, the settlement cannot: the call fails and the removal is rolled back
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::FillOrKill,
            &taker,
            1000,
            &eur,
            &usd,
            PRECISION,
            &Vec::from_array(&e, [id]),
        ),
        Some(709)
    );
    assert!(client.order(&id).is_some());
}

#[test]
fn test_expired_allowance_reads_as_zero() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    // a short-lived allowance backs the order at creation
    let expiry = e.ledger().sequence() + 10;
    token::Client::new(&e, &usd).approve(&maker, &axis, &1000, &expiry);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    advance_ledgers(&e, 20);
    assert_eq!(token::Client::new(&e, &usd).allowance(&maker, &axis), 0);
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (0, 0));
    assert!(client.order(&id).is_none());
}

#[test]
fn test_backing_is_shared_by_the_makers_orders() {
    // 1000 USD back two 600 USD orders: the second one only delivers what is left
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 1000);
    fund(&e, &eur, &axis, &taker, 10000);
    let first = store_order(&client, &maker, 600, &usd, &eur, PRECISION);
    let second = store_order(&client, &maker, 600, &usd, &eur, PRECISION);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1200,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [first, second]),
    );
    assert_eq!((sold, bought), (1000, 1000));
    assert!(client.order(&first).is_none());
    assert!(client.order(&second).is_none());
    assert_eq!(balance(&e, &usd, &maker), 0);
    assert_eq!(balance(&e, &eur, &maker), 1000);
}

#[test]
fn test_order_left_is_capped_by_the_backing_left() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 1000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 800, &usd, &eur, PRECISION);
    // 300 filled while fully backed: 500 left
    trade(
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
    assert_eq!(client.order(&id).unwrap().amount, 500);

    // the maker spends 300 elsewhere: 400 left in the wallet
    token::Client::new(&e, &usd).transfer(&maker, &taker, &300);
    trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        100,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    // 400 would be left on the order, but only 300 back it after the fill
    assert_eq!(client.order(&id).unwrap().amount, 300);
}

#[test]
fn test_trade_with_approval_grants_the_allowance() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    open_market(&client, &usd, &eur);
    token::StellarAssetClient::new(&e, &usd).mint(&trader, &10000);
    let live_until = super::setup::max_live_until(&e);

    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &trader,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
        &1,
        &0,
        &Some(crate::trade::Approval {
            asset: usd.clone(),
            amount: 5000,
            live_until,
        }),
    );
    assert!(id.is_some());
    assert_eq!(token::Client::new(&e, &usd).allowance(&trader, &axis), 5000);
}
