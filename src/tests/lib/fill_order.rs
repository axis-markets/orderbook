use super::setup::setup_test;
use crate::{SorobanOrderbook, SorobanOrderbookClient, PRECISION};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Vec};

#[test]
fn test_fill_order_empty_orders_list() {
    let (e, trader, _issuer, usd, eur) = setup_test();
    let contract_address = e.register(SorobanOrderbook, ());
    let client = SorobanOrderbookClient::new(&e, &contract_address);

    let arbitrageur = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);

    // Mint tokens
    usd_client.mint(&trader, &10000);

    // Create taker order
    let (_, _, taker_order_id) =
        client.sell_limit(&trader, &1000, &usd, &eur, &PRECISION, &100, &Vec::new(&e));

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
    let contract_address = e.register(SorobanOrderbook, ());
    let client = SorobanOrderbookClient::new(&e, &contract_address);

    let maker = Address::generate(&e);
    let eur_client = StellarAssetClient::new(&e, &eur);

    // Mint tokens
    eur_client.mint(&maker, &10000);

    // Create maker order
    let (_, _, maker_order_id) =
        client.sell_limit(&maker, &1000, &eur, &usd, &PRECISION, &100, &Vec::new(&e));

    // Try to fill non-existent taker order - should panic
    let orders = Vec::from_array(&e, [maker_order_id]);
    client.fill_order(&trader, &999, &orders);
}
