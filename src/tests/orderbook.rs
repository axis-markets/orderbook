//! Matching engine rounding: the taker's acquired amount is rounded down, the amount paid
//! for it rounded up, so a maker is never underpaid and the taker never pays more than one
//! unit above the exact price.
use super::lib::setup::{
    actor, fake_asset, fund, list_asset, no_orders, open_market, register_axis, setup_oracle,
    store_order, trade, try_trade, UNIT_PRICE,
};
use crate::math::invert_price_floor;
use crate::order::{OrderKind, TradeDirection};
use crate::orderbook::PRECISION;
use crate::AxisClient;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};
use test_case::test_case;

fn env() -> (Env, Address, Address, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    setup_oracle(&e, &issuer);
    list_asset(&e, &usd, UNIT_PRICE);
    list_asset(&e, &eur, UNIT_PRICE);
    let maker = actor(&e);
    let trader = actor(&e);
    (e, usd, eur, maker, trader)
}

//rounding
#[test_case(3, 2, 3000, 4501, 3000, 4500)]
#[test_case(3, 2, 3000, 4500, 3000, 4500)]
#[test_case(3, 2, 3000, 4499, 2999, 4499)]
#[test_case(3, 2, 2999, 4499, 2999, 4499)]
#[test_case(3, 2, 2999, 4498, 2998, 4497)]
#[test_case(2, 3, 3000, 2001, 3000, 2000)]
#[test_case(2, 3, 3000, 2000, 3000, 2000)]
#[test_case(2, 3, 3000, 1999, 2998, 1999)]
#[test_case(2, 3, 2999, 2000, 2999, 2000)]
#[test_case(2, 3, 2999, 1999, 2998, 1999)]
//micro trades
#[test_case(3, 2, 28, 27, 18, 27)]
#[test_case(3, 2, 28, 26, 17, 26)]
#[test_case(3, 2, 52, 51, 34, 51)]
#[test_case(3, 2, 52, 50, 33, 50)]
#[test_case(30000000, 2, 1, 1, 0, 0)]
#[test_case(30000000, 2, 10, 100000000, 6, 90000000)]
#[test_case(3, 20000000, 10000000, 10, 10000000, 2)]
#[test_case(3, 20000000, 10000000, 1, 6666666, 1)]
fn test_fill(
    n: i128,
    d: i128,
    x_order_amount: i128,
    y_trade_amount: i128,
    expected_x_received: i128,
    expected_y_sent: i128,
) {
    let (e, usd, eur, maker, trader) = env();
    let contract_address = register_axis(&e);
    let orderbook_client = AxisClient::new(&e, &contract_address);
    fund(&e, &usd, &contract_address, &maker, 10000000000000000);
    fund(&e, &eur, &contract_address, &trader, 10000000000000000);

    //maker sells X (USD) at n/d Y per X
    let price = PRECISION * n / d;
    let order_id = store_order(&orderbook_client, &maker, x_order_amount, &usd, &eur, price);
    let orders = Vec::from_array(&e, [order_id]);
    let (y_sent, x_received, _) = trade(
        &orderbook_client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &trader,
        y_trade_amount,
        &eur,
        &usd,
        invert_price_floor(&e, price),
        &orders,
    );
    assert_eq!(
        (y_sent, x_received),
        (expected_y_sent, expected_x_received),
        "trade {}/{} ({}X on order) {}Y -> ({}Y -> {}X)",
        n,
        d,
        x_order_amount,
        y_trade_amount,
        expected_y_sent,
        expected_x_received
    );
    //the maker never gets less than the order price for what they delivered
    assert!(y_sent * PRECISION >= x_received * price);
}

#[test]
fn test_dust_order_is_rejected() {
    //1 X at 1.5e-7 Y per X is worth less than one unit of Y: it can never be filled
    let (e, usd, eur, maker, _) = env();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    open_market(&client, &usd, &eur);
    fund(&e, &usd, &contract_address, &maker, 1000);
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &maker,
            1,
            &usd,
            &eur,
            PRECISION * 3 / 20000000,
            &no_orders(&e),
        ),
        Some(720)
    );
}
