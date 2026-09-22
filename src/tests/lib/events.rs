//! Verify that emitted contract events carry values matching what actually happened
//! on-chain: the right assets, amounts, addresses and the amount left on the order.
//!
//! Each expected event is built as a typed event struct and compared via
//! `Event::to_xdr`, after filtering `env.events().all()` down to the Axis
//! contract (SAC token events are excluded by `filter_by_contract`).

use super::setup::{
    actor, fake_asset, fund, no_orders, register_axis, remove_orders, setup_test, store_order,
    trade,
};
use crate::events::{OrderCreatedEvent, OrderUpdatedEvent, SwapEvent, TradeEvent};
use crate::order::{OrderKind, TradeDirection};
use crate::trade::TradeStep;
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events;
use soroban_sdk::{Address, Event, Vec};

// Cross rates reused by the swap tests.
const EUR_USD: i128 = 12 * PRECISION / 10; // 1.2 USD per EUR
const GBP_EUR: i128 = 11 * PRECISION / 10; // 1.1 EUR per GBP

#[allow(clippy::too_many_arguments)]
fn trade_event(
    order: u128,
    taker: &Address,
    maker: &Address,
    selling: &Address,
    buying: &Address,
    sold: i128,
    bought: i128,
    left: i128,
) -> TradeEvent {
    TradeEvent {
        selling: selling.clone(),
        buying: buying.clone(),
        order,
        taker: taker.clone(),
        maker: maker.clone(),
        sold,
        bought,
        left,
    }
}

#[test]
fn test_new_event_matches_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);
    fund(&e, &usd, &contract, &trader, 10000);

    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    // single event: the created order, with selling=USD, buying=EUR
    let expected = OrderCreatedEvent {
        selling: usd.clone(),
        buying: eur.clone(),
        id,
        owner: trader.clone(),
        price: PRECISION,
        amount: 1000,
        expires: 0,
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [expected.to_xdr(&e, &contract)]
    );
}

#[test]
fn test_trade_event_sell_match() {
    // Maker sells 1000 USD at 2 EUR/USD; taker sells 100 EUR -> buys 50 USD.
    let (e, maker, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);

    let taker = actor(&e);
    fund(&e, &usd, &contract, &maker, 10000);
    fund(&e, &eur, &contract, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, 2 * PRECISION);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        100,
        &eur,
        &usd,
        PRECISION / 2,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (100, 50));

    // `all()` reports only this (taker) invocation's events: one trade, no order event.
    // taker sold 100 EUR, bought 50 USD; maker order left with 950
    let expected = trade_event(id, &taker, &maker, &eur, &usd, 100, 50, 950);
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [expected.to_xdr(&e, &contract)]
    );
}

#[test]
fn test_trade_event_buy_match() {
    // Maker sells 1000 USD at 2 EUR/USD; buyer buys 100 USD -> pays 200 EUR.
    let (e, maker, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);

    let buyer = actor(&e);
    fund(&e, &usd, &contract, &maker, 10000);
    fund(&e, &eur, &contract, &buyer, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, 2 * PRECISION);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Fill,
        &buyer,
        100,
        &eur,
        &usd,
        2 * PRECISION,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (200, 100));

    let expected = trade_event(id, &buyer, &maker, &eur, &usd, 200, 100, 900);
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [expected.to_xdr(&e, &contract)]
    );
}

#[test]
fn test_trade_event_left_zero_on_full_fill() {
    // Maker offers only 100 USD at 2 EUR/USD; taker fully consumes it -> left = 0.
    let (e, maker, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);

    let taker = actor(&e);
    fund(&e, &usd, &contract, &maker, 10000);
    fund(&e, &eur, &contract, &taker, 10000);
    let id = store_order(&client, &maker, 100, &usd, &eur, 2 * PRECISION);

    trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION / 2,
        &Vec::from_array(&e, [id]),
    );

    // taker sells 200 EUR to claim all 100 USD -> maker order removed with 0 left
    let expected = trade_event(id, &taker, &maker, &eur, &usd, 200, 100, 0);
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [expected.to_xdr(&e, &contract)]
    );
    assert!(client.order(&id).is_none());
}

#[test]
fn test_trade_and_new_events_on_partial_fill_with_reminder() {
    let (e, maker, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);
    let taker = actor(&e);
    fund(&e, &usd, &contract, &maker, 10000);
    fund(&e, &eur, &contract, &taker, 10000);
    let id = store_order(&client, &maker, 300, &usd, &eur, PRECISION);

    let (_, _, created) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Limit,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [id]),
    );
    let fill = trade_event(id, &taker, &maker, &eur, &usd, 300, 300, 0);
    let new = OrderCreatedEvent {
        selling: eur.clone(),
        buying: usd.clone(),
        id: created.unwrap(),
        owner: taker.clone(),
        price: PRECISION,
        amount: 700,
        expires: 0,
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [fill.to_xdr(&e, &contract), new.to_xdr(&e, &contract)]
    );
}

#[test]
fn test_mod_event_on_removal() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);
    fund(&e, &usd, &contract, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, 2 * PRECISION);

    remove_orders(&client, &trader, &[id]);
    let expected = OrderUpdatedEvent {
        id,
        price: 2 * PRECISION,
        amount: 0,
        expires: 0,
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [expected.to_xdr(&e, &contract)]
    );
}

#[test]
fn test_no_events_when_nothing_matches() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);
    fund(&e, &usd, &contract, &trader, 10000);
    trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
        &no_orders(&e),
    );
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [] as [soroban_sdk::xdr::ContractEvent; 0]
    );
}

#[test]
fn test_swap_event_sell() {
    // Sell 1000 USD -> EUR -> GBP yields 757 GBP; the contract is the taker of the second hop.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);

    let maker1 = actor(&e);
    let maker2 = actor(&e);
    fund(&e, &eur, &contract, &maker1, 833);
    fund(&e, &gbp, &contract, &maker2, 757);
    fund(&e, &usd, &contract, &trader, 1000);

    let o1 = store_order(&client, &maker1, 833, &eur, &usd, EUR_USD);
    let o2 = store_order(&client, &maker2, 757, &gbp, &eur, GBP_EUR);

    let path = Vec::from_array(
        &e,
        [
            TradeStep {
                asset: eur.clone(),
                orders: Vec::from_array(&e, [o1]),
            },
            TradeStep {
                asset: gbp.clone(),
                orders: Vec::from_array(&e, [o2]),
            },
        ],
    );
    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &757,
        &path,
        &None,
    );
    assert_eq!((sold, bought), (1000, 757));

    // events for the swap invocation: 2 trades (front to back), 1 swap
    let trade1 = trade_event(o1, &trader, &maker1, &usd, &eur, 1000, 833, 0);
    let trade2 = trade_event(o2, &contract, &maker2, &eur, &gbp, 833, 757, 0);
    let swap = SwapEvent {
        selling: usd.clone(),
        buying: gbp.clone(),
        trader: trader.clone(),
        sold: 1000,
        bought: 757,
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [
            trade1.to_xdr(&e, &contract),
            trade2.to_xdr(&e, &contract),
            swap.to_xdr(&e, &contract),
        ]
    );
}

#[test]
fn test_swap_event_buy() {
    // Buy exactly 100 GBP via EUR using USD; costs 132 USD. The route is planned back to
    // front but executed front to back, so the EUR/USD leg settles first.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);

    let maker1 = actor(&e);
    let maker2 = actor(&e);
    fund(&e, &eur, &contract, &maker1, 110);
    fund(&e, &gbp, &contract, &maker2, 100);
    fund(&e, &usd, &contract, &trader, 200);

    let o1 = store_order(&client, &maker1, 110, &eur, &usd, EUR_USD);
    let o2 = store_order(&client, &maker2, 100, &gbp, &eur, GBP_EUR);

    let path = Vec::from_array(
        &e,
        [
            TradeStep {
                asset: eur.clone(),
                orders: Vec::from_array(&e, [o1]),
            },
            TradeStep {
                asset: gbp.clone(),
                orders: Vec::from_array(&e, [o2]),
            },
        ],
    );
    let (sold, bought) = client.swap(
        &TradeDirection::Buy,
        &trader,
        &usd,
        &200,
        &100,
        &path,
        &None,
    );
    assert_eq!((sold, bought), (132, 100));

    let trade1 = trade_event(o1, &trader, &maker1, &usd, &eur, 132, 110, 0);
    let trade2 = trade_event(o2, &contract, &maker2, &eur, &gbp, 110, 100, 0);
    let swap = SwapEvent {
        selling: usd.clone(),
        buying: gbp.clone(),
        trader: trader.clone(),
        sold: 132,
        bought: 100,
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [
            trade1.to_xdr(&e, &contract),
            trade2.to_xdr(&e, &contract),
            swap.to_xdr(&e, &contract),
        ]
    );
}
