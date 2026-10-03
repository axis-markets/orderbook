//! Faulty counterparties: a maker who cannot receive the taker's asset, or whose transfer fails
//! despite enough backing, is skipped with a `skip` event; a taker who cannot receive the asset
//! (a deauthorized or full trustline) fails the trade without flagging any maker.
use super::mock_token::{MockToken, MockTokenClient};
use super::setup::{
    actor, balance, code, fake_asset_revocable, fund, list_asset, nonce, open_market,
    register_axis, setup_test, store_order, trade, try_trade, UNIT_PRICE,
};
use crate::events::OrderSkippedEvent;
use crate::order::{order_id, OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Event, Vec};

/// USD replaced by an asset whose issuer can revoke authorization
fn revocable_usd(e: &Env, issuer: &Address) -> Address {
    let usd = fake_asset_revocable(e, issuer);
    list_asset(e, &usd, UNIT_PRICE);
    usd
}

#[test]
fn test_deauthorized_maker_is_skipped() {
    let (e, maker, issuer, _, eur) = setup_test();
    let usd = revocable_usd(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    // the balance and the allowance still read fine, but the transfer is refused
    StellarAssetClient::new(&e, &usd).set_authorized(&maker, &false);

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
    // the order stays untouched; the event flags the maker whose transfer failed
    assert_eq!(events, [OrderSkippedEvent { order: id }.to_xdr(&e, &axis)]);
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &eur, &taker), 10000);
}

#[test]
fn test_deauthorized_maker_does_not_block_the_others() {
    let (e, bad, issuer, _, eur) = setup_test();
    let usd = revocable_usd(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let good = actor(&e);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &bad, 10000);
    fund(&e, &usd, &axis, &good, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let bad_order = store_order(&client, &bad, 1000, &usd, &eur, PRECISION);
    let good_order = store_order(&client, &good, 1000, &usd, &eur, PRECISION);
    StellarAssetClient::new(&e, &usd).set_authorized(&bad, &false);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        2000,
        &eur,
        &usd,
        PRECISION,
        &Vec::from_array(&e, [bad_order, good_order]),
    );
    assert_eq!((sold, bought), (1000, 1000));
    assert_eq!(client.order(&bad_order).unwrap().amount, 1000);
    assert!(client.order(&good_order).is_none());
    assert_eq!(balance(&e, &eur, &good), 1000);
}

#[test]
fn test_taker_that_cannot_receive_is_rejected_first() {
    let (e, maker, issuer, _, eur) = setup_test();
    let usd = revocable_usd(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    // the taker is not allowed to hold USD
    StellarAssetClient::new(&e, &usd).set_authorized(&taker, &false);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Fill,
            &taker,
            1000,
            &eur,
            &usd,
            PRECISION,
            &Vec::from_array(&e, [id]),
        ),
        Some(708)
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &eur, &taker), 10000);
    assert_eq!(balance(&e, &usd, &maker), 10000);
}

#[test]
fn test_order_requires_the_trader_to_receive_the_bought_asset() {
    let (e, trader, issuer, _, eur) = setup_test();
    let usd = revocable_usd(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    open_market(&client, &usd, &eur);
    fund(&e, &eur, &axis, &trader, 10000);
    StellarAssetClient::new(&e, &usd).set_authorized(&trader, &false);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &trader,
            1000,
            &eur,
            &usd,
            PRECISION,
            &Vec::new(&e),
        ),
        Some(708)
    );
}

#[test]
fn test_maker_who_cannot_receive_is_skipped() {
    // Maker 1 is deauthorized for USD, the asset the taker pays with. Paying them would fail
    // after their EUR already moved, so their order is skipped before any transfer and maker 2
    // fills the trade instead
    let (e, bad, issuer, _, eur) = setup_test();
    let usd = revocable_usd(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let good = actor(&e);
    let taker = actor(&e);
    fund(&e, &eur, &axis, &bad, 1000);
    fund(&e, &eur, &axis, &good, 1000);
    fund(&e, &usd, &axis, &taker, 10000);
    let bad_order = store_order(&client, &bad, 1000, &eur, &usd, PRECISION);
    let good_order = store_order(&client, &good, 1000, &eur, &usd, PRECISION);
    StellarAssetClient::new(&e, &usd).set_authorized(&bad, &false);

    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        2000,
        &usd,
        &eur,
        PRECISION,
        &Vec::from_array(&e, [bad_order, good_order]),
    );
    let events = e.events().all().filter_by_contract(&axis);
    assert_eq!((sold, bought), (1000, 1000));
    assert!(events
        .events()
        .contains(&OrderSkippedEvent { order: bad_order }.to_xdr(&e, &axis)));
    assert_eq!(client.order(&bad_order).unwrap().amount, 1000);
    assert!(client.order(&good_order).is_none());
    assert_eq!(balance(&e, &eur, &bad), 1000);
    assert_eq!(balance(&e, &usd, &good), 1000);
}

/// A token whose holders can be capped like a classic trustline limit, quoted at `UNIT_PRICE`
fn capped_asset(e: &Env) -> (Address, MockTokenClient<'_>) {
    let asset = e.register(MockToken, ());
    list_asset(e, &asset, UNIT_PRICE);
    (asset.clone(), MockTokenClient::new(e, &asset))
}

#[test]
fn test_taker_with_full_trustline_is_rejected() {
    // the taker's line has room for 500 only: the fill fails on the taker's side, so the trade
    // fails instead of flagging the maker
    let (e, maker, _, _, eur) = setup_test();
    let (capped, capped_client) = capped_asset(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &capped, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &capped, &eur, PRECISION);
    capped_client.set_limit(&taker, &500);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Fill,
            &taker,
            1000,
            &eur,
            &capped,
            PRECISION,
            &Vec::from_array(&e, [id]),
        ),
        Some(708)
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &eur, &taker), 10000);
    assert_eq!(balance(&e, &capped, &maker), 10000);
}

#[test]
fn test_taker_with_room_for_one_maker_is_rejected() {
    // the first maker settles, the second does not fit: nothing settles and no maker is flagged
    let (e, first, _, _, eur) = setup_test();
    let (capped, capped_client) = capped_asset(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let second = actor(&e);
    let taker = actor(&e);
    fund(&e, &capped, &axis, &first, 1000);
    fund(&e, &capped, &axis, &second, 1000);
    fund(&e, &eur, &axis, &taker, 10000);
    let first_order = store_order(&client, &first, 1000, &capped, &eur, PRECISION);
    let second_order = store_order(&client, &second, 1000, &capped, &eur, PRECISION);
    capped_client.set_limit(&taker, &1000);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Fill,
            &taker,
            2000,
            &eur,
            &capped,
            PRECISION,
            &Vec::from_array(&e, [first_order, second_order]),
        ),
        Some(708)
    );
    assert_eq!(client.order(&first_order).unwrap().amount, 1000);
    assert_eq!(client.order(&second_order).unwrap().amount, 1000);
    assert_eq!(balance(&e, &capped, &taker), 0);
    assert_eq!(balance(&e, &eur, &taker), 10000);
}

#[test]
fn test_limit_trade_with_full_trustline_creates_no_order() {
    // with every maker flagged, the whole amount would have become an order crossing the book
    let (e, maker, _, _, eur) = setup_test();
    let (capped, capped_client) = capped_asset(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let taker = actor(&e);
    fund(&e, &capped, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 10000);
    let id = store_order(&client, &maker, 1000, &capped, &eur, PRECISION);
    capped_client.set_limit(&taker, &0);

    let taker_nonce = nonce();
    let res = client.try_trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &taker,
        &1000,
        &eur,
        &capped,
        &PRECISION,
        &Vec::from_array(&e, [id]),
        &taker_nonce,
        &0,
        &None,
    );
    assert_eq!(code(res), Some(708));
    let taker_order = e.as_contract(&axis, || order_id(&e, &taker, taker_nonce));
    assert!(client.order(&taker_order).is_none());
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}
