use super::setup::{fake_asset, setup_test};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};
use test_case::test_case;

#[test]
fn test_buy_limit_creates_order() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Mint selling tokens (EUR) to the buyer
    let eur_client = StellarAssetClient::new(&e, &eur);
    eur_client.mint(&trader, &10000);

    let initial_balance = eur_client.balance(&trader);
    let amount = 1000;
    // Create buy limit order
    let (sold, bought, order_id) = client.trade(
        &TradeDirection::Buy,
        &OrderKind::Limit,
        &trader,
        &amount,
        &eur,
        &usd,
        &PRECISION,
        &Vec::new(&e),
    );

    // No matching orders provided -> nothing executed
    assert_eq!(sold, 0);
    assert_eq!(bought, 0);
    assert_eq!(order_id, 1);

    // Verify the remainder order was created in sell-equivalent form:
    //   selling = trader's selling token (EUR)
    //   buying  = trader's buying token (USD)
    //   amount  = amount * price / PRECISION (the worst-case deposit)
    //   price   = invert(user.price) -> stored as buying-per-selling
    let order = client.order(&order_id).unwrap();
    assert_eq!(order.amount, amount);
    assert_eq!(order.selling, eur);
    assert_eq!(order.buying, usd);
    assert_eq!(order.price, PRECISION);
    assert_eq!(order.owner, trader);

    // Selling tokens moved trader -> contract as the deposit
    let trader_balance = eur_client.balance(&trader);
    let contract_balance = eur_client.balance(&contract_address);
    assert_eq!(trader_balance, initial_balance - amount);
    assert_eq!(contract_balance, amount);
}

#[test]
#[should_panic(expected = "#10")]
fn test_buy_limit_insufficient_balance() {
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    // Trader has less than the worst-case deposit (amount * price / PRECISION = 1000)
    let eur_client = StellarAssetClient::new(&e, &eur);
    eur_client.mint(&trader, &100);

    client.trade(
        &TradeDirection::Buy,
        &OrderKind::Limit,
        &trader,
        &1000,
        &eur,
        &usd,
        &PRECISION,
        &Vec::new(&e),
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

    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let eur_client = StellarAssetClient::new(&e, &eur);
    eur_client.mock_all_auths().mint(&trader, &10000);

    // Trader auth not provided -> should panic
    client.trade(
        &TradeDirection::Buy,
        &OrderKind::Limit,
        &trader,
        &1000,
        &eur,
        &usd,
        &PRECISION,
        &Vec::new(&e),
    );
}

#[test_case(OrderKind::FillOrKill, 300, 0, 0, 0; "Fill-or-Kill, price too low, failed trade")]
#[test_case(OrderKind::FillOrKill, 30000, 1, 0, 0; "Fill-or-Kill, insufficient liquidity, failed trade")]
#[test_case(OrderKind::FillOrKill, 1000, 1, 1000, 1000; "Fill-or-Kill, success at exact price")]
#[test_case(OrderKind::FillOrKill, 1000, 2, 1000, 1000; "Fill-or-Kill, success with refund")]
#[test_case(OrderKind::Fill, 300, 0, 0, 0; "Fill, price too low, failed trade")]
#[test_case(OrderKind::Fill, 300, 1, 300, 300; "Fill, partial execution at exact price")]
#[test_case(OrderKind::Fill, 300, 2, 300, 300; "Fill, partial execution with refund")]
fn test_buy_fill(
    kind: OrderKind,
    amount: i128,
    price_multiplier: i128,
    expected_sold: i128,
    expected_bought: i128,
) {
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let taker = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);

    // Maker funds in USD (to sell), taker funds in EUR (to pay)
    usd_client.mint(&maker, &10000);
    eur_client.mint(&taker, &10000);

    // Maker creates a sell order: 1000 USD at price PRECISION (= 1 EUR per USD)
    let (_, _, order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );

    // Taker tries to buy `amount` USD with EUR at max price = price_multiplier * PRECISION
    let (sold, bought, created_order) = client.trade(
        &TradeDirection::Buy,
        &kind,
        &taker,
        &amount,
        &eur,
        &usd,
        &(price_multiplier * PRECISION),
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!(sold, expected_sold);
    assert_eq!(bought, expected_bought);
    assert_eq!(created_order, 0);

    // Maker deposited 1000 USD into the contract when creating the sell order, so
    // their wallet balance stays at 9000 USD regardless of whether the buy matched
    // (their USD flows contract -> taker, not maker -> taker).
    assert_eq!(usd_client.balance(&maker), 9000);
    assert_eq!(eur_client.balance(&maker), sold);

    assert_eq!(usd_client.balance(&taker), bought);
    assert_eq!(eur_client.balance(&taker), 10000 - sold);
}

#[test]
fn test_buy_full_fill_refund() {
    // Maker sells USD at 1 EUR/USD; buyer offers max 2 EUR/USD.
    // Buyer should pay the maker's better price, not the worst-case max.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let buyer = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);

    usd_client.mint(&maker, &10000);
    eur_client.mint(&buyer, &10000);

    // Maker: sell 1000 USD at price PRECISION (1 EUR/USD)
    let (_, _, order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );

    // Buyer: buy 500 USD at max 2 EUR/USD
    let (sold, bought, created_order) = client.trade(
        &TradeDirection::Buy,
        &OrderKind::Fill,
        &buyer,
        &500,
        &eur,
        &usd,
        &(2 * PRECISION),
        &Vec::from_array(&e, [order_id]),
    );

    // Buyer acquired exactly 500 USD; paid only 500 EUR (at maker price = 1 EUR/USD),
    // not 1000 EUR which would be the buyer's worst-case cost at max price.
    assert_eq!(bought, 500);
    assert_eq!(sold, 500);
    assert_eq!(created_order, 0);

    // Maker order should still have 500 USD remaining
    let remaining = client.order(&order_id).unwrap();
    assert_eq!(remaining.amount, 500);

    // Balances reflect actual amounts (refund implicit — only `sold` EUR moved)
    assert_eq!(usd_client.balance(&buyer), 500);
    assert_eq!(eur_client.balance(&buyer), 9500);
    assert_eq!(eur_client.balance(&maker), 500);
}

#[test]
fn test_buy_partial_creates_remainder() {
    // Maker has limited liquidity; buyer's Limit order should fill what's available
    // and leave a remainder order on the book.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let buyer = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);

    usd_client.mint(&maker, &10000);
    eur_client.mint(&buyer, &10000);

    // Maker: sell only 300 USD at price PRECISION
    let (_, _, maker_order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &300,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );

    // Buyer wants 500 USD at max 1 EUR/USD, Limit kind
    let (sold, bought, remainder_id) = client.trade(
        &TradeDirection::Buy,
        &OrderKind::Limit,
        &buyer,
        &500,
        &eur,
        &usd,
        &PRECISION,
        &Vec::from_array(&e, [maker_order_id]),
    );

    // 300 USD filled from maker; 200 USD remaining as a buy order on the book
    assert_eq!(bought, 300);
    assert_eq!(sold, 300);
    assert_eq!(remainder_id, 2);

    // Maker order fully consumed -> removed from the book
    assert!(client.order(&maker_order_id).is_none());

    // Remainder order stored as sell-equivalent: selling=EUR, buying=USD,
    // amount = remaining_buy * max_price / PRECISION = 200 * PRECISION / PRECISION = 200 EUR,
    // price = invert(max_price) = PRECISION
    let remainder = client.order(&remainder_id).unwrap();
    assert_eq!(remainder.selling, eur);
    assert_eq!(remainder.buying, usd);
    assert_eq!(remainder.amount, 200);
    assert_eq!(remainder.price, PRECISION);
    assert_eq!(remainder.owner, buyer);

    // Buyer: paid 300 EUR to maker + 200 EUR deposited for the remainder = 500 EUR
    assert_eq!(eur_client.balance(&buyer), 9500);
    assert_eq!(usd_client.balance(&buyer), 300);
    assert_eq!(eur_client.balance(&maker), 300);
    assert_eq!(usd_client.balance(&maker), 9700);
    // Contract holds the 200 EUR deposit for the remainder buy order
    assert_eq!(eur_client.balance(&contract_address), 200);
}

#[test]
fn test_buy_at_non_unit_price() {
    // Maker sells USD wanting 2 EUR per USD; buyer buys 100 USD and must pay 200 EUR.
    // Confirms buy_amounts_for stays correct at a non-1:1 price.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let buyer = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);

    usd_client.mint(&maker, &10000);
    eur_client.mint(&buyer, &10000);

    // Maker: sell 1000 USD at price 2*PRECISION (2 EUR per USD)
    let (_, _, order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &1000,
        &usd,
        &eur,
        &(2 * PRECISION),
        &Vec::new(&e),
    );

    // Buyer: buy 100 USD at max 2 EUR/USD
    let (sold, bought, created_order) = client.trade(
        &TradeDirection::Buy,
        &OrderKind::Fill,
        &buyer,
        &100,
        &eur,
        &usd,
        &(2 * PRECISION),
        &Vec::from_array(&e, [order_id]),
    );

    // 100 USD at 2 EUR/USD costs 200 EUR
    assert_eq!(sold, 200);
    assert_eq!(bought, 100);
    assert_eq!(created_order, 0);

    // Maker order partially consumed: 1000 - 100 = 900 USD remaining
    let remaining = client.order(&order_id).unwrap();
    assert_eq!(remaining.amount, 900);

    assert_eq!(usd_client.balance(&buyer), 100);
    assert_eq!(eur_client.balance(&buyer), 9800);
    assert_eq!(eur_client.balance(&maker), 200);
    assert_eq!(usd_client.balance(&maker), 9000);
}
