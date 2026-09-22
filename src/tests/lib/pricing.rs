//! Minimum order size valuation and oracle price caching.
use super::setup::{
    actor, advance, clear_price, ensure_market, fake_asset, fund, no_orders, open_market,
    oracle_address, oracle_client, order_update, register_axis, register_oracle, set_price,
    setup_test, trade, try_trade, update_one, MIN_TRADE_SIZE, ORACLE_DAILY_FEE, ORACLE_DECIMALS,
    START_TIMESTAMP, TOKEN_DECIMALS, UNIT_PRICE,
};
use crate::order::{OrderKind, TradeDirection};
use crate::orderbook::PRECISION;
use crate::pricing::{PriceCache, MAX_PRICE_AGE};
use crate::reflector_beam::Asset;
use crate::AxisClient;
use soroban_sdk::{Address, Env, Vec};

/// Oracle price of 1 USD per whole token
const ONE_USD: i128 = 10i128.pow(ORACLE_DECIMALS);
/// Token base units in one whole token
const ONE_TOKEN: i128 = 10i128.pow(TOKEN_DECIMALS);

fn limit(
    client: &AxisClient,
    direction: TradeDirection,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
) -> u128 {
    let e = client.env.clone();
    ensure_market(client, selling, buying);
    let (_, _, id) = trade(
        client,
        direction,
        OrderKind::Limit,
        trader,
        amount,
        selling,
        buying,
        price,
        &no_orders(&e),
    );
    id.expect("the order must be created")
}

/// Attempt a limit trade, returning the contract error code on failure
fn try_limit(
    client: &AxisClient,
    direction: TradeDirection,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
) -> Option<u32> {
    let e = client.env.clone();
    ensure_market(client, selling, buying);
    try_trade(
        client,
        direction,
        OrderKind::Limit,
        trader,
        amount,
        selling,
        buying,
        price,
        &no_orders(&e),
    )
}

fn cached_price(e: &Env, axis: &Address, asset: &Address) -> Option<PriceCache> {
    e.as_contract(axis, || e.storage().temporary().get(asset))
}

#[test]
fn test_min_order_size_sell_boundary() {
    let (e, trader, _, usd, eur) = setup_test();
    // 1 USD per whole USD token: the minimum order sells exactly one token
    set_price(&e, &usd, ONE_USD);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10 * ONE_TOKEN);

    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            ONE_TOKEN - 1,
            &usd,
            &eur,
            PRECISION
        ),
        Some(720)
    );
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            ONE_TOKEN,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );
}

#[test]
fn test_min_order_size_values_whole_trade_not_remainder() {
    let (e, maker, _, usd, eur) = setup_test();
    set_price(&e, &usd, ONE_USD);
    set_price(&e, &eur, ONE_USD);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &maker, 10 * ONE_TOKEN);
    // maker creates 1 USD order for EUR at 1
    let order = limit(
        &client,
        TradeDirection::Sell,
        &maker,
        ONE_TOKEN,
        &usd,
        &eur,
        PRECISION,
    );

    // taker sells 1.5 EUR tokens: 1 fills the maker, the 0.5 remainder order created although it is
    // below the minimum on its own, because the trade as a whole is above it
    let taker = actor(&e);
    let amount = ONE_TOKEN + ONE_TOKEN / 2;
    fund(&e, &eur, &axis, &taker, amount);
    let (sold, bought, id) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Limit,
        &taker,
        amount,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [order]),
    );
    assert_eq!(sold, ONE_TOKEN);
    assert_eq!(bought, ONE_TOKEN);
    assert_eq!(client.order(&id.unwrap()).unwrap().amount, ONE_TOKEN / 2);
}

#[test]
fn test_min_order_size_buy_uses_selling_amount() {
    let (e, trader, _, usd, eur) = setup_test();
    // 1 USD per whole EUR token, so a stroop of EUR is worth next to nothing
    set_price(&e, &eur, ONE_USD);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &eur, &axis, &trader, 10 * ONE_TOKEN);

    // buying 1 USD stroop at half price costs 1 EUR stroop (rounded up): far below 1 USD
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Buy,
            &trader,
            1,
            &eur,
            &usd,
            PRECISION / 2
        ),
        Some(720)
    );
    // buying 2 USD tokens costs 1 EUR token = 1 USD
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Buy,
            &trader,
            2 * ONE_TOKEN,
            &eur,
            &usd,
            PRECISION / 2
        ),
        None
    );
}

#[test]
fn test_min_order_size_uses_buying_side_when_selling_unlisted() {
    let (e, trader, issuer, usd, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &gbp, &axis, &trader, 1000);

    // 1 GBP stroop at half price is worth less than one USD stroop: dust, never fillable
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1,
            &gbp,
            &usd,
            PRECISION / 2
        ),
        Some(720)
    );
    // 1 GBP stroop buying 1 USD stroop (1 USD at unit price) passes
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1,
            &gbp,
            &usd,
            PRECISION
        ),
        None
    );
    // known limitation: with the selling asset unlisted the order price set by the trader
    // drives the valuation, so an absurdly priced dust order passes
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1,
            &gbp,
            &usd,
            1_000_000 * PRECISION
        ),
        None
    );
}

#[test]
fn test_trade_and_update_never_call_the_oracle() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);

    // opening the market caches the prices of both listed assets
    open_market(&client, &usd, &eur);
    assert_eq!(oracle_client(&e).calls(), 2);
    let cached = cached_price(&e, &axis, &usd).unwrap();
    assert_eq!(cached.price, UNIT_PRICE);
    assert_eq!(cached.timestamp, START_TIMESTAMP);
    assert!(cached_price(&e, &axis, &eur).is_some());

    // the quotes disappear and the cache ages to its limit: orders are valued from the cache
    clear_price(&e, &usd);
    clear_price(&e, &eur);
    advance(&e, MAX_PRICE_AGE);
    let id = limit(
        &client,
        TradeDirection::Sell,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
    );
    update_one(&client, &trader, order_update(id, 800, PRECISION));
    assert_eq!(
        oracle_client(&e).calls(),
        2,
        "trade and update made no oracle call"
    );
}

#[test]
fn test_requote_caches_fresh_prices() {
    let (e, _, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    open_market(&client, &usd, &eur);

    advance(&e, 60);
    set_price(&e, &usd, 2 * UNIT_PRICE);
    client.requote(&usd, &eur);

    assert_eq!(oracle_client(&e).calls(), 4);
    let cached = cached_price(&e, &axis, &usd).unwrap();
    assert_eq!(cached.price, 2 * UNIT_PRICE);
    assert_eq!(cached.timestamp, START_TIMESTAMP + 60);
}

#[test]
fn test_trade_values_at_the_cached_price_until_requoted() {
    let (e, trader, _, usd, eur) = setup_test();
    // 1 USD per whole USD token: the minimum order sells one token
    set_price(&e, &usd, ONE_USD);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 100 * ONE_TOKEN);
    open_market(&client, &usd, &eur);

    // USD drops tenfold on the oracle: the cache still values one token at 1 USD
    set_price(&e, &usd, ONE_USD / 10);
    let sell = |amount| {
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            amount,
            &usd,
            &eur,
            PRECISION,
        )
    };
    assert_eq!(sell(ONE_TOKEN), None);

    // a requote brings the new quote in: one token is now worth 0.1 USD
    client.requote(&usd, &eur);
    assert_eq!(sell(ONE_TOKEN), Some(720));
    assert_eq!(sell(10 * ONE_TOKEN), None);
}

#[test]
fn test_failed_fetch_keeps_the_cached_price() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    open_market(&client, &usd, &eur);

    // the quote disappears: the requote still succeeds and the last known price stays
    advance(&e, 60);
    clear_price(&e, &usd);
    assert!(client.requote(&usd, &eur).is_some());
    assert_eq!(oracle_client(&e).calls(), 4);
    let cached = cached_price(&e, &axis, &usd).unwrap();
    assert_eq!(cached.price, UNIT_PRICE);
    assert_eq!(cached.timestamp, START_TIMESTAMP);
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );
}

#[test]
fn test_price_older_than_max_age_is_rejected() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    limit(
        &client,
        TradeDirection::Sell,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
    );
    clear_price(&e, &usd);

    // the cached price still backs orders while it stays within the safe age
    advance(&e, MAX_PRICE_AGE);
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );

    // past it the order has no trustworthy valuation
    advance(&e, 1);
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        Some(722)
    );
}

#[test]
#[should_panic(expected = "#722")]
fn test_missing_quote_fails() {
    let (e, trader, _, usd, eur) = setup_test();
    clear_price(&e, &usd);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    limit(
        &client,
        TradeDirection::Sell,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
    );
}

#[test]
#[should_panic(expected = "#722")]
fn test_non_positive_quote_fails() {
    let (e, trader, _, usd, eur) = setup_test();
    set_price(&e, &usd, 0);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    limit(
        &client,
        TradeDirection::Sell,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
    );
}

#[test]
fn test_lapsed_access_fails_until_market_funded() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    limit(
        &client,
        TradeDirection::Sell,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
    );

    // oracle access lapses: the cached price keeps backing orders while it stays usable
    oracle_client(&e).set_access(&axis, &Asset::Stellar(usd.clone()), &(START_TIMESTAMP - 1));
    advance(&e, 60);
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );

    // once it ages out there is nothing left to value orders against, and a requote cannot
    // read the fresh quote without access
    advance(&e, MAX_PRICE_AGE);
    set_price(&e, &usd, UNIT_PRICE);
    client.requote(&usd, &eur);
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        Some(722)
    );

    // subsidizing buys access and caches the quote again
    client.subsidize(&trader, &usd, &eur, &(2 * ORACLE_DAILY_FEE));
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );
}

#[test]
fn test_price_cached_under_a_previous_oracle_is_not_reused() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);

    //opening the market caches the USD price
    limit(
        &client,
        TradeDirection::Sell,
        &trader,
        1000,
        &usd,
        &eur,
        PRECISION,
    );
    let cached: PriceCache = e.as_contract(&axis, || e.storage().temporary().get(&usd).unwrap());
    assert_eq!(cached.oracle, oracle_address(&e));

    //hand over to an oracle that quotes nothing: the stale record must not stand in for a quote
    client.set_oracle(&register_oracle(&e, ORACLE_DECIMALS));
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION
        ),
        Some(722)
    );
}

#[test]
fn test_min_order_size_follows_the_configured_limit() {
    let (e, trader, _, usd, eur) = setup_test();
    // 1 USD per whole USD token, so the threshold in tokens equals the configured USD value
    set_price(&e, &usd, ONE_USD);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 100 * ONE_TOKEN);

    //the default 1 USD minimum lets one whole token through
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            ONE_TOKEN,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );

    //raise the floor to 10 USD: the same order is now too small, ten tokens are not
    client.set_floor(&(10 * MIN_TRADE_SIZE));
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            10 * ONE_TOKEN - 1,
            &usd,
            &eur,
            PRECISION
        ),
        Some(720)
    );
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            10 * ONE_TOKEN,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );
}

#[test]
fn test_min_order_size_accepts_a_fractional_limit() {
    let (e, trader, _, usd, eur) = setup_test();
    set_price(&e, &usd, ONE_USD);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10 * ONE_TOKEN);

    //half a USD, below the whole-USD granularity the limit used to be stuck at
    client.set_floor(&(MIN_TRADE_SIZE / 2));
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            ONE_TOKEN / 2 - 1,
            &usd,
            &eur,
            PRECISION
        ),
        Some(720)
    );
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            ONE_TOKEN / 2,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );
}

#[test]
fn test_zero_min_trade_size_disables_the_check() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10 * ONE_TOKEN);
    //open the market while the limit still applies, then switch the limit off
    limit(
        &client,
        TradeDirection::Sell,
        &trader,
        ONE_TOKEN,
        &usd,
        &eur,
        PRECISION,
    );
    client.set_floor(&0);

    //a single stroop order created, and the oracle is not consulted at all: no quote is needed
    clear_price(&e, &usd);
    clear_price(&e, &eur);
    let calls = oracle_client(&e).calls();
    assert_eq!(
        try_limit(
            &client,
            TradeDirection::Sell,
            &trader,
            1,
            &usd,
            &eur,
            PRECISION
        ),
        None
    );
    assert_eq!(oracle_client(&e).calls(), calls, "no price was fetched");
}
