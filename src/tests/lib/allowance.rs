//! Orders are backed by the maker's balance and allowance instead of a deposit: a fill the
//! maker cannot cover is skipped with a `skip` event, and the order is left unchanged.
use super::setup::{
    actor, advance_ledgers, approve, assert_no_custody, balance, fund, open_market, register_axis,
    setup_test, store_order, trade, try_trade,
};
use crate::events::{OrderSkippedEvent, TradeEvent};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token, Event, Vec};

#[test]
fn test_partially_backed_order_serves_covered_fills_only() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    // the maker lowers the allowance below the order
    approve(&e, &usd, &axis, &maker, 400);

    // a fill of 300 is covered by the allowance of 400
    let (sold, bought, created) = trade(
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
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!((sold, bought, created), (300, 300, None));
    // the order keeps its full remaining amount, it is not capped to the backing left
    assert_eq!(client.order(&id).unwrap().amount, 700);
    let expected = TradeEvent {
        selling: eur.clone(),
        buying: usd.clone(),
        order: id,
        taker: taker.clone(),
        maker: maker.clone(),
        sold: 300,
        bought: 300,
        left: 700,
    };
    assert_eq!(events, [expected.to_xdr(&e, &axis)]);

    // a fill of 600 exceeds the 100 left: the order is skipped and left unchanged
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        600,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!((sold, bought), (0, 0));
    assert_eq!(client.order(&id).unwrap().amount, 700);
    assert_eq!(events, [OrderSkippedEvent { order: id }.to_xdr(&e, &axis)]);
    assert_eq!(balance(&e, &usd, &maker), 9700);
    assert_eq!(balance(&e, &eur, &maker), 300);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

#[test]
fn test_fill_beyond_the_balance_skips_the_order() {
    let (e, maker, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 1000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, 2 * PRECISION);
    // the maker moves most of the balance away: 300 USD left
    token::Client::new(&e, &usd).transfer(&maker, &taker, &700);

    // the whole order (1000 USD for 2000 EUR) is more than the maker holds
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
    assert_eq!((sold, bought), (0, 0));
    assert_eq!(client.order(&id).unwrap().amount, 1000);

    // 300 USD at 2 EUR/USD cost 600 EUR, which the balance covers
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        600,
        &eur,
        &usd,
        PRECISION / 2,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (600, 300));
    assert_eq!(client.order(&id).unwrap().amount, 700);
    assert_eq!(balance(&e, &usd, &maker), 0);
    assert_eq!(balance(&e, &eur, &maker), 600);
}

#[test]
fn test_unbacked_order_is_skipped() {
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
    // the order stays for the indexer to judge; the event flags the maker
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(events, [OrderSkippedEvent { order: id }.to_xdr(&e, &axis)]);
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

    // the maker cannot back the fill, so nothing is planned and the call fails
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
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}

#[test]
fn test_backing_is_shared_by_the_makers_orders() {
    // 1000 USD back two 600 USD orders: once the first is filled only 400 are left, which
    // does not cover the second fill
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
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!((sold, bought), (600, 600));
    assert!(client.order(&first).is_none());
    assert_eq!(client.order(&second).unwrap().amount, 600);
    assert!(events
        .events()
        .contains(&OrderSkippedEvent { order: second }.to_xdr(&e, &axis)));
    assert_eq!(balance(&e, &usd, &maker), 400);
    assert_eq!(balance(&e, &eur, &maker), 600);
}

#[test]
fn test_order_left_is_not_capped_by_the_backing_left() {
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
    // 400 left on the order, although only 300 back it after the fill
    assert_eq!(client.order(&id).unwrap().amount, 400);
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
