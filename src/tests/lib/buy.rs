use super::setup::{
    actor, assert_no_custody, balance, fake_asset, fund, no_orders, open_market, register_axis,
    setup_oracle, setup_test, store_order, trade, try_trade,
};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};
use test_case::test_case;

#[test]
fn test_buy_limit_creates_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    open_market(&client, &usd, &eur);
    fund(&e, &eur, &contract_address, &trader, 10000);

    let amount = 1000;
    // Create buy limit order
    let (sold, bought, order_id) = trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        amount,
        &eur,
        &usd,
        PRECISION,
        &no_orders(&e),
    );

    // No matching orders provided -> nothing executed
    assert_eq!(sold, 0);
    assert_eq!(bought, 0);
    let order_id = order_id.unwrap();

    // Verify the remainder order was created in sell-equivalent form:
    //   selling = trader's selling token (EUR)
    //   buying  = trader's buying token (USD)
    //   amount  = ceil(amount * price / PRECISION) (the worst-case cost)
    //   price   = invert(user.price) -> stored as buying-per-selling
    let order = client.order(&order_id).unwrap();
    assert_eq!(order.amount, amount);
    assert_eq!(order.selling, eur);
    assert_eq!(order.buying, usd);
    assert_eq!(order.price, PRECISION);
    assert_eq!(order.owner, trader);

    // Nothing moved
    assert_eq!(balance(&e, &eur, &trader), 10000);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test]
#[should_panic(expected = "#702")]
fn test_buy_limit_insufficient_balance() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    open_market(&client, &usd, &eur);

    // Trader has less than the worst-case cost (amount * price / PRECISION = 1000)
    fund(&e, &eur, &contract_address, &trader, 100);

    trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        1000,
        &eur,
        &usd,
        PRECISION,
        &no_orders(&e),
    );
}

#[test]
#[should_panic(expected = "\"contract call failed\", trade")]
fn test_buy_limit_requires_auth() {
    let e = Env::default();
    // Do NOT mock auth
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    setup_oracle(&e, &issuer);

    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    StellarAssetClient::new(&e, &eur)
        .mock_all_auths()
        .mint(&trader, &10000);

    // Trader auth not provided -> should panic
    trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        1000,
        &eur,
        &usd,
        PRECISION,
        &no_orders(&e),
    );
}

#[test_case(OrderKind::FillOrKill, 1000, 10, 1000, 1000; "Fill-or-Kill, success at exact price")]
#[test_case(OrderKind::FillOrKill, 1000, 20, 1000, 1000; "Fill-or-Kill, success at better price")]
#[test_case(OrderKind::Fill, 300, 5, 0, 0; "Fill, price too low, no trade")]
#[test_case(OrderKind::Fill, 300, 10, 300, 300; "Fill, partial execution at exact price")]
#[test_case(OrderKind::Fill, 300, 20, 300, 300; "Fill, partial execution at better price")]
fn test_buy_fill(
    kind: OrderKind,
    amount: i128,
    price_tenths: i128,
    expected_sold: i128,
    expected_bought: i128,
) {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    // Maker funds in USD (to sell), taker funds in EUR (to pay)
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);

    // Maker creates a sell order: 1000 USD at price PRECISION (= 1 EUR per USD)
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    // Taker tries to buy `amount` USD with EUR at max price = price_tenths / 10 * PRECISION
    let (sold, bought, created_order) = trade(
        &client,
        TradeDirection::Buy,
        kind,
        &taker,
        amount,
        &eur,
        &usd,
        price_tenths * PRECISION / 10,
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!(sold, expected_sold);
    assert_eq!(bought, expected_bought);
    assert_eq!(created_order, None);

    assert_eq!(balance(&e, &usd, &maker), 10000 - bought);
    assert_eq!(balance(&e, &eur, &maker), sold);
    assert_eq!(balance(&e, &usd, &taker), bought);
    assert_eq!(balance(&e, &eur, &taker), 10000 - sold);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test_case(300, 5; "Fill-or-Kill, price too low")]
#[test_case(30000, 10; "Fill-or-Kill, insufficient liquidity")]
fn test_buy_fill_or_kill_not_filled(amount: i128, price_tenths: i128) {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Buy,
            OrderKind::FillOrKill,
            &taker,
            amount,
            &eur,
            &usd,
            price_tenths * PRECISION / 10,
            &Vec::from_array(&e, [order_id]),
        ),
        Some(709)
    );
    assert_eq!(balance(&e, &eur, &taker), 10000);
    assert_eq!(client.order(&order_id).unwrap().amount, 1000);
}

#[test]
fn test_buy_full_fill_at_better_price() {
    // Maker sells USD at 1 EUR/USD; buyer offers max 2 EUR/USD.
    // Buyer pays the maker's better price, not the worst-case max.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let buyer = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &buyer, 10000);

    // Maker: sell 1000 USD at price PRECISION (1 EUR/USD)
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    // Buyer: buy 500 USD at max 2 EUR/USD
    let (sold, bought, created_order) = trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Fill,
        &buyer,
        500,
        &eur,
        &usd,
        2 * PRECISION,
        &Vec::from_array(&e, [order_id]),
    );

    // Buyer acquired exactly 500 USD; paid only 500 EUR (at maker price = 1 EUR/USD)
    assert_eq!(bought, 500);
    assert_eq!(sold, 500);
    assert_eq!(created_order, None);

    // Maker order should still have 500 USD remaining
    let remaining = client.order(&order_id).unwrap();
    assert_eq!(remaining.amount, 500);

    assert_eq!(balance(&e, &usd, &buyer), 500);
    assert_eq!(balance(&e, &eur, &buyer), 9500);
    assert_eq!(balance(&e, &eur, &maker), 500);
}

#[test]
fn test_buy_partial_creates_remainder() {
    // Maker has limited liquidity; buyer's Limit order should fill what's available
    // and leave a remainder order on the book.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let buyer = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &buyer, 10000);

    // Maker: sell only 300 USD at price PRECISION
    let maker_order_id = store_order(&client, &maker, 300, &usd, &eur, PRECISION);

    // Buyer wants 500 USD at max 1 EUR/USD, Limit kind
    let (sold, bought, remainder_id) = trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &buyer,
        500,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [maker_order_id]),
    );

    // 300 USD filled from maker; 200 USD remaining as a buy order on the book
    assert_eq!(bought, 300);
    assert_eq!(sold, 300);
    let remainder_id = remainder_id.unwrap();

    // Maker order fully consumed -> removed from the book
    assert!(client.order(&maker_order_id).is_none());

    // Remainder order stored as sell-equivalent: selling=EUR, buying=USD,
    // amount = ceil(remaining_buy * max_price / PRECISION) = 200 EUR, price = invert(max_price)
    let remainder = client.order(&remainder_id).unwrap();
    assert_eq!(remainder.selling, eur);
    assert_eq!(remainder.buying, usd);
    assert_eq!(remainder.amount, 200);
    assert_eq!(remainder.price, PRECISION);
    assert_eq!(remainder.owner, buyer);

    // Buyer paid 300 EUR to the maker, the remainder stays in the wallet
    assert_eq!(balance(&e, &eur, &buyer), 9700);
    assert_eq!(balance(&e, &usd, &buyer), 300);
    assert_eq!(balance(&e, &eur, &maker), 300);
    assert_eq!(balance(&e, &usd, &maker), 9700);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test]
fn test_buy_at_non_unit_price() {
    // Maker sells USD wanting 2 EUR per USD; buyer buys 100 USD and must pay 200 EUR.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let buyer = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &buyer, 10000);

    // Maker: sell 1000 USD at price 2*PRECISION (2 EUR per USD)
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, 2 * PRECISION);

    // Buyer: buy 100 USD at max 2 EUR/USD
    let (sold, bought, created_order) = trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Fill,
        &buyer,
        100,
        &eur,
        &usd,
        2 * PRECISION,
        &Vec::from_array(&e, [order_id]),
    );

    // 100 USD at 2 EUR/USD costs 200 EUR
    assert_eq!(sold, 200);
    assert_eq!(bought, 100);
    assert_eq!(created_order, None);

    // Maker order partially consumed: 1000 - 100 = 900 USD remaining
    let remaining = client.order(&order_id).unwrap();
    assert_eq!(remaining.amount, 900);

    assert_eq!(balance(&e, &usd, &buyer), 100);
    assert_eq!(balance(&e, &eur, &buyer), 9800);
    assert_eq!(balance(&e, &eur, &maker), 200);
    assert_eq!(balance(&e, &usd, &maker), 9900);
}

#[test]
fn test_buy_remainder_rounds_against_the_buyer() {
    // Buy 1000 USD at max 1/3 EUR per USD: the remainder order as ceil(333.3) = 334 EUR
    // at the rounded-up inverse price, so a fill at the stored price never underpays
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    open_market(&client, &usd, &eur);
    fund(&e, &eur, &contract_address, &trader, 10000);

    let (_, _, id) = trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        1000,
        &eur,
        &usd,
        PRECISION / 3,
        &no_orders(&e),
    );
    let order = client.order(&id.unwrap()).unwrap();
    assert_eq!(order.amount, 334);
    // invert_price_ceil(333333333333333333): 10^36 / 333333333333333333 leaves a remainder
    assert_eq!(order.price, 3_000_000_000_000_000_004);
}
