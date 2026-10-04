//! Contract instance and code lifetime: keepers extend it with the `ExtendFootprintTTL` operation, a
//! regular call extends it only when keepers let it fall below 3 days.
use super::setup::{
    advance_ledgers, fund, mainnet_ttls, register_axis, setup_test, store_order, MIN_PERSISTENT_TTL,
};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Deployer as _;
use soroban_sdk::{Address, Env};

const LPD: u32 = 17_280;

fn instance_ttl(e: &Env, axis: &Address) -> u32 {
    e.deployer().get_contract_instance_ttl(axis)
}

fn code_ttl(e: &Env, axis: &Address) -> u32 {
    e.deployer().get_contract_code_ttl(axis)
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
