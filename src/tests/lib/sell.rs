use super::setup::{
    actor, assert_no_custody, balance, fake_asset, fund, list_asset, no_orders, open_market,
    register_axis, setup_oracle, setup_test, store_order, trade, try_trade, UNIT_PRICE,
};
use crate::order::{order_id as derive_order_id, OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};
use test_case::test_case;

#[test]
fn test_sell_limit_creates_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    open_market(&client, &usd, &eur);
    fund(&e, &usd, &contract_address, &trader, 10000);

    let amount = 1000;
    let (sold, bought, order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &trader,
        &amount,
        &usd,
        &eur,
        &PRECISION,
        &no_orders(&e),
        &7,
        &0,
        &None,
    );

    // Should not be filled (no matching orders)
    assert_eq!(sold, 0);
    assert_eq!(bought, 0);
    let order_id = order_id.unwrap();
    assert_eq!(order_id, derive_order_id(&e, &trader, 7));

    // Verify order was created
    let order = client.order(&order_id).unwrap();
    assert_eq!(order.amount, amount);
    assert_eq!(order.owner, trader);
    // Nothing moved: the order is backed by the trader's balance and allowance
    assert_eq!(balance(&e, &usd, &trader), 10000);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test]
#[should_panic(expected = "#702")]
fn test_sell_limit_insufficient_balance() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    // Don't mint enough tokens
    fund(&e, &usd, &contract_address, &trader, 100);

    // Try to create order for more than balance - should panic
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
}

#[test]
#[should_panic(expected = "#703")]
fn test_sell_limit_insufficient_allowance() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    // Enough tokens, but nothing approved to the contract
    StellarAssetClient::new(&e, &usd).mint(&trader, &10000);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
}

#[test]
#[should_panic(expected = "\"contract call failed\", trade")]
fn test_sell_limit_requires_auth() {
    let e = Env::default();
    // Don't mock auth
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    setup_oracle(&e, &issuer);

    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    list_asset(&e, &usd, UNIT_PRICE);
    e.mock_all_auths();
    open_market(&client, &usd, &eur);
    e.set_auths(&[]);

    StellarAssetClient::new(&e, &usd)
        .mock_all_auths()
        .mint(&trader, &10000);

    // This should panic because trader auth is not provided
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
}

#[test_case(OrderKind::FillOrKill, 1000, 1, 1000, 1000; "Fill-or-Kill, success")]
#[test_case(OrderKind::Fill, 300, 10, 0, 0; "Fill, higher price, no trade")]
#[test_case(OrderKind::Fill, 300, 1, 300, 300; "Fill, partial execution, successful trade")]
fn test_sell_fill(
    kind: OrderKind,
    amount: i128,
    price: i128,
    expected_sold: i128,
    expected_bought: i128,
) {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);

    // Create a large order
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    let (sold, bought, created_order) = trade(
        &client,
        TradeDirection::Sell,
        kind,
        &taker,
        amount,
        &eur,
        &usd,
        price * PRECISION,
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!(sold, expected_sold);
    assert_eq!(bought, expected_bought);
    assert_eq!(created_order, None);

    // Check balances after the trade: the maker pays out of their wallet
    assert_eq!(balance(&e, &usd, &maker), 10000 - bought);
    assert_eq!(balance(&e, &eur, &maker), sold);
    assert_eq!(balance(&e, &usd, &taker), bought);
    assert_eq!(balance(&e, &eur, &taker), 10000 - sold);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test_case(300, 10; "Fill-or-Kill, higher price")]
#[test_case(30000, 1; "Fill-or-Kill, insufficient liquidity")]
fn test_sell_fill_or_kill_not_filled(amount: i128, price: i128) {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    // FillOrKill fails instead of executing partially
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::FillOrKill,
            &taker,
            amount,
            &eur,
            &usd,
            price * PRECISION,
            &Vec::from_array(&e, [order_id]),
        ),
        Some(709)
    );
    // nothing moved, the maker order is untouched
    assert_eq!(balance(&e, &eur, &taker), 10000);
    assert_eq!(balance(&e, &usd, &taker), 0);
    assert_eq!(client.order(&order_id).unwrap().amount, 1000);
}

#[test]
fn test_sell_at_non_unit_price() {
    // Maker sells USD wanting 2 EUR per USD; taker sells EUR to buy USD.
    // Selling 100 EUR at 2 EUR/USD must yield 50 USD (a price-inverted impl would yield 200).
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);

    // Maker: sell 1000 USD at price 2*PRECISION (2 EUR per USD)
    let order_id = store_order(&client, &maker, 1000, &usd, &eur, 2 * PRECISION);

    // Taker: sell 100 EUR to buy USD, accepting down to 0.5 USD per EUR (max_price = PRECISION/2)
    let (sold, bought, created_order) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        100,
        &eur,
        &usd,
        PRECISION / 2,
        &Vec::from_array(&e, [order_id]),
    );

    // 100 EUR at 2 EUR/USD buys 50 USD
    assert_eq!(sold, 100);
    assert_eq!(bought, 50);
    assert_eq!(created_order, None);

    // Maker order partially consumed: 1000 - 50 = 950 USD remaining
    let remaining = client.order(&order_id).unwrap();
    assert_eq!(remaining.amount, 950);

    assert_eq!(balance(&e, &usd, &maker), 9950);
    assert_eq!(balance(&e, &eur, &maker), 100);
    assert_eq!(balance(&e, &usd, &taker), 50);
    assert_eq!(balance(&e, &eur, &taker), 9900);
}

#[test]
fn test_sell_at_non_unit_price_clamped() {
    // Maker offers only 100 USD at 2 EUR/USD; taker tries to sell 1000 EUR.
    // The order caps the fill: taker sells 200 EUR to claim the maker's 100 USD.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);

    // Maker: sell only 100 USD at price 2*PRECISION
    let order_id = store_order(&client, &maker, 100, &usd, &eur, 2 * PRECISION);

    // Taker: sell 1000 EUR to buy USD (more than the order can supply)
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION / 2,
        &Vec::from_array(&e, [order_id]),
    );

    // Order holds only 100 USD; acquiring it costs 200 EUR
    assert_eq!(sold, 200);
    assert_eq!(bought, 100);

    // Maker order fully consumed -> removed from the book
    assert!(client.order(&order_id).is_none());

    assert_eq!(balance(&e, &eur, &maker), 200);
    assert_eq!(balance(&e, &usd, &maker), 9900);
    assert_eq!(balance(&e, &usd, &taker), 100);
    assert_eq!(balance(&e, &eur, &taker), 9800);
}

#[test]
fn test_sell_limit_partial_fill_stores_remainder() {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);
    let order_id = store_order(&client, &maker, 300, &usd, &eur, PRECISION);

    // taker sells 1000 EUR: 300 fill, 700 stored
    let (sold, bought, id) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Limit,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!((sold, bought), (300, 300));
    let remainder = client.order(&id.unwrap()).unwrap();
    assert_eq!(remainder.amount, 700);
    assert_eq!(remainder.selling, eur);
    assert_eq!(remainder.buying, usd);
    assert_eq!(remainder.price, PRECISION);
    assert!(client.order(&order_id).is_none());
    // the remainder stays in the wallet
    assert_eq!(balance(&e, &eur, &taker), 9700);
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test]
fn test_self_trade_is_allowed() {
    // A trader crossing their own order moves nothing net and clears the order
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &trader, 10000);
    fund(&e, &eur, &contract_address, &trader, 10000);
    let order_id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    let (sold, bought, created) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &trader,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!((sold, bought, created), (1000, 1000, None));
    assert!(client.order(&order_id).is_none());
    assert_eq!(balance(&e, &usd, &trader), 10000);
    assert_eq!(balance(&e, &eur, &trader), 10000);
}

#[test]
fn test_duplicate_order_id_fills_once() {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);
    let order_id = store_order(&client, &maker, 300, &usd, &eur, PRECISION);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        1000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [order_id, order_id, order_id]),
    );
    assert_eq!((sold, bought), (300, 300));
    assert_eq!(balance(&e, &usd, &maker), 9700);
}
