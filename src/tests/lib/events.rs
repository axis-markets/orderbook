//! Verify that emitted contract events (`TradeEvent`, `OrderEvent`, `SwapEvent`)
//! carry values matching what actually happened on-chain: the right action,
//! assets, amounts, and taker/maker addresses.
//!
//! Each expected event is built as a typed event struct and compared via
//! `Event::to_xdr`, after filtering `env.events().all()` down to the Axis
//! contract (SAC token-transfer events are excluded by `filter_by_contract`).

use super::setup::{fake_asset, setup_test};
use crate::events::{OrderEvent, SwapEvent, TradeEvent};
use crate::order::{Order, OrderKind, TradeDirection};
use crate::trade::{Swap, Trade, TradeStep};
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, token::StellarAssetClient, Address, Event, Symbol, Vec};

// Cross rates reused by the swap tests.
const EUR_USD: i128 = 12 * PRECISION / 10; // 1.2 USD per EUR
const GBP_EUR: i128 = 11 * PRECISION / 10; // 1.1 EUR per GBP

fn limit_order(
    id: u64,
    selling: &Address,
    buying: &Address,
    amount: i128,
    quote: i128,
    owner: &Address,
    price: i128,
) -> Order {
    Order {
        id,
        kind: OrderKind::Limit,
        selling: selling.clone(),
        buying: buying.clone(),
        amount,
        quote,
        owner: owner.clone(),
        price,
        expires: 0,
    }
}

fn order_event(action: Symbol, order: Order) -> OrderEvent {
    OrderEvent {
        action,
        selling: order.selling.clone(),
        buying: order.buying.clone(),
        order,
    }
}

fn trade_event(
    id: u64,
    order: u64,
    taker: &Address,
    maker: &Address,
    selling: &Address,
    buying: &Address,
    sold: i128,
    bought: i128,
) -> TradeEvent {
    TradeEvent {
        selling: selling.clone(),
        buying: buying.clone(),
        trade: Trade {
            id,
            order,
            taker: taker.clone(),
            maker: maker.clone(),
            selling: selling.clone(),
            buying: buying.clone(),
            sold,
            bought,
        },
    }
}

#[test]
fn test_order_event_created_matches_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract);

    StellarAssetClient::new(&e, &usd).mint(&trader, &10000);

    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &trader,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );

    // single event: the created order, with selling=USD, buying=EUR
    let expected = order_event(
        symbol_short!("created"),
        limit_order(id, &usd, &eur, 1000, 1000, &trader, PRECISION),
    );
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [expected.to_xdr(&e, &contract)]
    );
}

#[test]
fn test_trade_event_sell_match() {
    // Maker sells 1000 USD at 2 EUR/USD; taker sells 100 EUR -> buys 50 USD.
    let (e, maker, _, usd, eur) = setup_test();
    let contract = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract);

    let taker = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);
    usd_client.mint(&maker, &10000);
    eur_client.mint(&taker, &10000);

    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &1000,
        &usd,
        &eur,
        &(2 * PRECISION),
        &Vec::new(&e),
    );

    let (sold, bought, _) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &taker,
        &100,
        &eur,
        &usd,
        &(PRECISION / 2),
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (100, 50));

    // `all()` reports only this (taker) invocation's events: [trade, updated].
    // taker sold 100 EUR, bought 50 USD; maker order updated to 950 remaining
    let trade = trade_event(1, id, &taker, &maker, &eur, &usd, 100, 50);
    let updated = order_event(
        symbol_short!("updated"),
        limit_order(id, &usd, &eur, 950, 1000, &maker, 2 * PRECISION),
    );
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [trade.to_xdr(&e, &contract), updated.to_xdr(&e, &contract)]
    );

    // event amounts agree with the tokens that actually moved
    assert_eq!(eur_client.balance(&maker), 100);
    assert_eq!(usd_client.balance(&taker), 50);
}

#[test]
fn test_trade_event_buy_match() {
    // Maker sells 1000 USD at 2 EUR/USD; buyer buys 100 USD -> pays 200 EUR.
    let (e, maker, _, usd, eur) = setup_test();
    let contract = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract);

    let buyer = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);
    usd_client.mint(&maker, &10000);
    eur_client.mint(&buyer, &10000);

    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &1000,
        &usd,
        &eur,
        &(2 * PRECISION),
        &Vec::new(&e),
    );

    let (sold, bought, _) = client.trade(
        &TradeDirection::Buy,
        &OrderKind::Fill,
        &buyer,
        &100,
        &eur,
        &usd,
        &(2 * PRECISION),
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (200, 100));

    // buyer paid 200 EUR, received 100 USD; maker order updated to 900 remaining
    let trade = trade_event(1, id, &buyer, &maker, &eur, &usd, 200, 100);
    let updated = order_event(
        symbol_short!("updated"),
        limit_order(id, &usd, &eur, 900, 1000, &maker, 2 * PRECISION),
    );
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [trade.to_xdr(&e, &contract), updated.to_xdr(&e, &contract)]
    );

    assert_eq!(eur_client.balance(&maker), 200);
    assert_eq!(usd_client.balance(&buyer), 100);
}

#[test]
fn test_order_event_removed_on_full_fill() {
    // Maker offers only 100 USD at 2 EUR/USD; taker fully consumes it -> order removed.
    let (e, maker, _, usd, eur) = setup_test();
    let contract = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract);

    let taker = Address::generate(&e);
    StellarAssetClient::new(&e, &usd).mint(&maker, &10000);
    StellarAssetClient::new(&e, &eur).mint(&taker, &10000);

    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &100,
        &usd,
        &eur,
        &(2 * PRECISION),
        &Vec::new(&e),
    );

    client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &taker,
        &1000,
        &eur,
        &usd,
        &(PRECISION / 2),
        &Vec::from_array(&e, [id]),
    );

    // taker sells 200 EUR to claim all 100 USD -> maker order removed with 0 remaining
    let trade = trade_event(1, id, &taker, &maker, &eur, &usd, 200, 100);
    let removed = order_event(
        symbol_short!("removed"),
        limit_order(id, &usd, &eur, 0, 100, &maker, 2 * PRECISION),
    );
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [trade.to_xdr(&e, &contract), removed.to_xdr(&e, &contract)]
    );
    assert!(client.order(&id).is_none());
}

#[test]
fn test_swap_event_sell() {
    // Sell 1000 USD -> EUR -> GBP yields 757 GBP; contract is the per-leg taker.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let contract = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract);

    let maker1 = Address::generate(&e);
    let maker2 = Address::generate(&e);
    StellarAssetClient::new(&e, &eur).mint(&maker1, &833);
    StellarAssetClient::new(&e, &gbp).mint(&maker2, &757);
    StellarAssetClient::new(&e, &usd).mint(&trader, &1000);

    let (_, _, o1) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker1,
        &833,
        &eur,
        &usd,
        &EUR_USD,
        &Vec::new(&e),
    );
    let (_, _, o2) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker2,
        &757,
        &gbp,
        &eur,
        &GBP_EUR,
        &Vec::new(&e),
    );

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
    let (sold, bought) = client.swap(&TradeDirection::Sell, &trader, &usd, &1000, &757, &path);
    assert_eq!((sold, bought), (1000, 757));

    // events for the swap invocation: 2 trades (taker = contract), 2 removed, 1 swap
    let trade1 = trade_event(1, o1, &contract, &maker1, &usd, &eur, 1000, 833);
    let trade2 = trade_event(2, o2, &contract, &maker2, &eur, &gbp, 833, 757);
    let swap = SwapEvent {
        selling: usd.clone(),
        buying: gbp.clone(),
        swap: Swap {
            id: 3,
            trader: trader.clone(),
            selling: usd.clone(),
            buying: gbp.clone(),
            sold: 1000,
            bought: 757,
        },
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [
            trade1.to_xdr(&e, &contract),
            trade2.to_xdr(&e, &contract),
            order_event(
                symbol_short!("removed"),
                limit_order(o1, &eur, &usd, 0, 833, &maker1, EUR_USD)
            )
            .to_xdr(&e, &contract),
            order_event(
                symbol_short!("removed"),
                limit_order(o2, &gbp, &eur, 0, 757, &maker2, GBP_EUR)
            )
            .to_xdr(&e, &contract),
            swap.to_xdr(&e, &contract),
        ]
    );

    assert_eq!(StellarAssetClient::new(&e, &gbp).balance(&trader), 757);
}

#[test]
fn test_swap_event_buy() {
    // Buy exactly 100 GBP via EUR using USD; costs 132 USD. Buy walks back to front,
    // so the GBP/EUR leg settles (and its order is removed) before the EUR/USD leg.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let contract = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract);

    let maker1 = Address::generate(&e);
    let maker2 = Address::generate(&e);
    StellarAssetClient::new(&e, &eur).mint(&maker1, &110);
    StellarAssetClient::new(&e, &gbp).mint(&maker2, &100);
    StellarAssetClient::new(&e, &usd).mint(&trader, &200);

    let (_, _, o1) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker1,
        &110,
        &eur,
        &usd,
        &EUR_USD,
        &Vec::new(&e),
    );
    let (_, _, o2) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker2,
        &100,
        &gbp,
        &eur,
        &GBP_EUR,
        &Vec::new(&e),
    );

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
    let (sold, bought) = client.swap(&TradeDirection::Buy, &trader, &usd, &200, &100, &path);
    assert_eq!((sold, bought), (132, 100));

    // GBP/EUR leg (order2) settles first: trade id 1, removed first.
    let trade1 = trade_event(1, o2, &contract, &maker2, &eur, &gbp, 110, 100);
    let trade2 = trade_event(2, o1, &contract, &maker1, &usd, &eur, 132, 110);
    let swap = SwapEvent {
        selling: usd.clone(),
        buying: gbp.clone(),
        swap: Swap {
            id: 3,
            selling: usd.clone(),
            buying: gbp.clone(),
            trader: trader.clone(),
            sold: 132,
            bought: 100,
        },
    };
    assert_eq!(
        e.events().all().filter_by_contract(&contract),
        [
            trade1.to_xdr(&e, &contract),
            trade2.to_xdr(&e, &contract),
            order_event(
                symbol_short!("removed"),
                limit_order(o2, &gbp, &eur, 0, 100, &maker2, GBP_EUR)
            )
            .to_xdr(&e, &contract),
            order_event(
                symbol_short!("removed"),
                limit_order(o1, &eur, &usd, 0, 110, &maker1, EUR_USD)
            )
            .to_xdr(&e, &contract),
            swap.to_xdr(&e, &contract),
        ]
    );

    assert_eq!(StellarAssetClient::new(&e, &gbp).balance(&trader), 100);
}
