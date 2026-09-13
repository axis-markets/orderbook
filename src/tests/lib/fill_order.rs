use super::setup::setup_test;
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Vec};

#[test]
fn test_fill_order_empty_orders_list() {
    let (e, trader, _issuer, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let arbitrageur = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);

    // Mint tokens
    usd_client.mint(&trader, &10000);

    // Create taker order
    let (_, _, taker_order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &trader,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );

    // Try to fill with empty orders list
    let orders = Vec::new(&e);
    let (sold, bought) = client.fill_order(&arbitrageur, &taker_order_id, &orders);

    // Should not execute any trades
    assert_eq!(sold, 0);
    assert_eq!(bought, 0);

    // Taker order should still exist with original amount
    let taker_order = client.order(&taker_order_id).unwrap();
    assert_eq!(taker_order.amount, 1000);
}

#[test]
#[should_panic]
fn test_fill_order_taker_not_found() {
    let (e, trader, _issuer, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let maker = Address::generate(&e);
    let eur_client = StellarAssetClient::new(&e, &eur);

    // Mint tokens
    eur_client.mint(&maker, &10000);

    // Create maker order
    let (_, _, maker_order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &1000,
        &eur,
        &usd,
        &PRECISION,
        &Vec::new(&e),
    );

    // Try to fill non-existent taker order - should panic
    let orders = Vec::from_array(&e, [maker_order_id]);
    client.fill_order(&trader, &999, &orders);
}

/// Taker sells 1000 USD for EUR at price 1, maker sells `maker_amount` EUR for USD at price 1.
/// Returns (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id).
fn setup_cross<'a>(
    e: &'a soroban_sdk::Env,
    usd: &Address,
    eur: &Address,
    maker_amount: i128,
) -> (AxisClient<'a>, Address, Address, Address, u64, u64) {
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(e, &contract_address);

    let taker_owner = Address::generate(e);
    let maker = Address::generate(e);
    let arbitrageur = Address::generate(e);

    StellarAssetClient::new(e, usd).mint(&taker_owner, &1000);
    StellarAssetClient::new(e, eur).mint(&maker, &maker_amount);

    let (_, _, taker_order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &taker_owner,
        &1000,
        usd,
        eur,
        &PRECISION,
        &Vec::new(e),
    );
    let (_, _, maker_order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &maker_amount,
        eur,
        usd,
        &PRECISION,
        &Vec::new(e),
    );
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
fn test_fill_order_partially_fills_taker() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, 400);
    let usd_token = soroban_sdk::token::Client::new(&e, &usd);
    let eur_token = soroban_sdk::token::Client::new(&e, &eur);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought) = client.fill_order(&arbitrageur, &taker_order_id, &orders);
    assert_eq!(sold, 400);
    assert_eq!(bought, 400);

    // taker order must be decremented by the sold amount
    let taker_order = client.order(&taker_order_id).unwrap();
    assert_eq!(taker_order.amount, 600);
    // maker order executed in full and removed
    assert!(client.order(&maker_order_id).is_none());

    // arbitrageur nets to zero, counterparties receive their tokens
    assert_eq!(usd_token.balance(&arbitrageur), 0);
    assert_eq!(eur_token.balance(&arbitrageur), 0);
    assert_eq!(eur_token.balance(&taker_owner), 400);
    assert_eq!(usd_token.balance(&maker), 400);
    // contract still holds the taker's remaining deposit
    assert_eq!(usd_token.balance(&client.address), 600);
    assert_eq!(eur_token.balance(&client.address), 0);
}

#[test]
fn test_fill_order_fully_fills_taker() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, taker_owner, maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, 1500);
    let usd_token = soroban_sdk::token::Client::new(&e, &usd);
    let eur_token = soroban_sdk::token::Client::new(&e, &eur);

    let orders = Vec::from_array(&e, [maker_order_id]);
    let (sold, bought) = client.fill_order(&arbitrageur, &taker_order_id, &orders);
    assert_eq!(sold, 1000);
    assert_eq!(bought, 1000);

    // taker order executed in full and removed
    assert!(client.order(&taker_order_id).is_none());
    // maker keeps the remainder
    let maker_order = client.order(&maker_order_id).unwrap();
    assert_eq!(maker_order.amount, 500);

    assert_eq!(usd_token.balance(&arbitrageur), 0);
    assert_eq!(eur_token.balance(&arbitrageur), 0);
    assert_eq!(eur_token.balance(&taker_owner), 1000);
    assert_eq!(usd_token.balance(&maker), 1000);
    assert_eq!(usd_token.balance(&client.address), 0);
    assert_eq!(eur_token.balance(&client.address), 500);
}

#[test]
fn test_fill_order_requires_trader_auth() {
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let (client, arbitrageur, _taker_owner, _maker, taker_order_id, maker_order_id) =
        setup_cross(&e, &usd, &eur, 400);

    // drop the blanket auth mock: nobody authorizes the call now
    e.set_auths(&[]);
    let orders = Vec::from_array(&e, [maker_order_id]);
    let res = client.try_fill_order(&arbitrageur, &taker_order_id, &orders);
    assert!(
        res.is_err(),
        "fill_order must fail without trader authorization"
    );

    // nothing changed on the book
    assert_eq!(client.order(&taker_order_id).unwrap().amount, 1000);
    assert_eq!(client.order(&maker_order_id).unwrap().amount, 400);
}
