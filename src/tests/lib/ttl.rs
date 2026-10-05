//! Entry lifetimes. Keepers extend the contract instance and code with the `ExtendFootprintTTL`
//! operation, a regular call extends them only when keepers let them fall below 3 days. Writing an
//! order to a market extends the market to outlive the order. Lifetimes are converted into ledgers
//! with the configured ledger close time.
use super::setup::{
    advance_ledgers, fund, mainnet_ttls, open_market, order_update, register_axis, setup_test,
    store_order, update_one, LPD, MIN_PERSISTENT_TTL,
};
use crate::market::{canonical, DataKey};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::storage::{Persistent as _, Temporary as _};
use soroban_sdk::testutils::Deployer as _;
use soroban_sdk::{Address, Env};

/// Ledgers per day at 4 seconds per ledger
const LPD_4S: u32 = 86_400 / 4;

fn instance_ttl(e: &Env, axis: &Address) -> u32 {
    e.deployer().get_contract_instance_ttl(axis)
}

fn code_ttl(e: &Env, axis: &Address) -> u32 {
    e.deployer().get_contract_code_ttl(axis)
}

fn market_ttl(e: &Env, axis: &Address, x: &Address, y: &Address) -> u32 {
    let (base, quote) = canonical(x, y);
    e.as_contract(axis, || {
        e.storage()
            .persistent()
            .get_ttl(&DataKey::Market(base, quote))
    })
}

fn order_ttl(e: &Env, axis: &Address, id: u128) -> u32 {
    e.as_contract(axis, || e.storage().persistent().get_ttl(&id))
}

fn price_ttl(e: &Env, axis: &Address, asset: &Address) -> u32 {
    e.as_contract(axis, || e.storage().temporary().get_ttl(asset))
}

/// Contract with mainnet TTLs, aged until just under `days` of instance lifetime are left
/// (at most 120, the lifetime granted at deployment)
fn aged_contract(days: u32) -> (Env, Address, Address, Address, Address) {
    let (e, trader, _, usd, eur) = setup_test();
    mainnet_ttls(&e);
    let axis = register_axis(&e);
    assert_eq!(instance_ttl(&e, &axis), MIN_PERSISTENT_TTL - 1);
    advance_ledgers(&e, MIN_PERSISTENT_TTL - days * LPD);
    assert_eq!(instance_ttl(&e, &axis), days * LPD - 1);
    (e, axis, trader, usd, eur)
}

#[test]
fn test_trading_leaves_a_long_lifetime_alone() {
    // 29 days left: the keeper's job, a trader pays no contract rent
    let (e, axis, trader, usd, eur) = aged_contract(29);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(instance_ttl(&e, &axis), 29 * LPD - 1);
    assert_eq!(code_ttl(&e, &axis), 29 * LPD - 1);
}

#[test]
fn test_trading_tops_up_a_short_lifetime() {
    // under 2 days left: the keepers failed, a regular call extends to 3 days
    let (e, axis, trader, usd, eur) = aged_contract(2);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(instance_ttl(&e, &axis), 3 * LPD);
    assert_eq!(code_ttl(&e, &axis), 3 * LPD);
}

#[test]
fn test_order_written_to_a_market_keeps_it_alive() {
    // The market entry was created 60 days ago and has 60 days left, an order created now lives
    // 120: writing the order extends the market a day beyond the order lifetime
    let (e, trader, _, usd, eur) = setup_test();
    mainnet_ttls(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    // months pass below: no floor, so the orders need no cached price
    client.set_floor(&0);
    open_market(&client, &usd, &eur);
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), MIN_PERSISTENT_TTL - 1);

    advance_ledgers(&e, 60 * LPD);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(order_ttl(&e, &axis, id), MIN_PERSISTENT_TTL - 1);
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), 121 * LPD);

    // within a day the market still covers a new order and is left alone, then it is due again
    advance_ledgers(&e, LPD - 1);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), 120 * LPD + 1);
    advance_ledgers(&e, 1);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), 121 * LPD);
}

#[test]
fn test_order_update_keeps_the_market_alive() {
    let (e, trader, _, usd, eur) = setup_test();
    mainnet_ttls(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    client.set_floor(&0);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);

    // 100 days later the update extends the order to 120 days and the market a day beyond
    advance_ledgers(&e, 100 * LPD);
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), 21 * LPD);
    update_one(&client, &trader, order_update(id, 900, PRECISION));
    assert_eq!(order_ttl(&e, &axis, id), 120 * LPD);
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), 121 * LPD);
}

#[test]
fn test_ledger_time_scales_the_lifetimes() {
    // With 4-second ledgers a day takes 21,600 ledgers instead of 17,280
    let (e, trader, _, usd, eur) = setup_test();
    mainnet_ttls(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    client.set_ledger_time(&4);
    fund(&e, &usd, &axis, &trader, 10000);

    let id = store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    // the market outlives the order by a day
    assert_eq!(market_ttl(&e, &axis, &usd, &eur), 121 * LPD_4S);
    // a cached price entry covers the 72 hours the price stays usable
    assert_eq!(price_ttl(&e, &axis, &usd), 3 * LPD_4S);
    // an update without expiration extends the order to 120 days
    update_one(&client, &trader, order_update(id, 900, PRECISION));
    assert_eq!(order_ttl(&e, &axis, id), 120 * LPD_4S);
}

#[test]
fn test_contract_top_up_follows_the_ledger_time() {
    let (e, axis, trader, usd, eur) = aged_contract(2);
    let client = AxisClient::new(&e, &axis);
    client.set_ledger_time(&4);
    fund(&e, &usd, &axis, &trader, 10000);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert_eq!(instance_ttl(&e, &axis), 3 * LPD_4S);
    assert_eq!(code_ttl(&e, &axis), 3 * LPD_4S);
}
