//! Contract instance and code lifetime.
use super::setup::{
    advance_ledgers, fund, mainnet_ttls, register_axis, setup_test, store_order, MAX_ENTRY_TTL,
    MIN_PERSISTENT_TTL,
};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::Deployer as _;
use soroban_sdk::{Address, Env};

const LPD: u32 = 17_280;

fn instance_ttl(e: &Env, axis: &Address) -> u32 {
    e.deployer().get_contract_instance_ttl(axis)
}

/// Contract with mainnet TTLs, aged until less than 30 days of instance lifetime are left
fn aged_contract() -> (Env, Address, Address, Address, Address) {
    let (e, trader, _, usd, eur) = setup_test();
    mainnet_ttls(&e);
    let axis = register_axis(&e);
    assert_eq!(instance_ttl(&e, &axis), MIN_PERSISTENT_TTL - 1);
    advance_ledgers(&e, MIN_PERSISTENT_TTL - 20 * LPD);
    assert!(instance_ttl(&e, &axis) < 30 * LPD);
    (e, axis, trader, usd, eur)
}

#[test]
fn test_keepalive_extends_instance_and_code() {
    let (e, axis, _, _, _) = aged_contract();
    let client = AxisClient::new(&e, &axis);
    client.keepalive();
    // extended to 180 days, capped by the network maximum
    let ttl = instance_ttl(&e, &axis);
    assert!(
        (179 * LPD..=MAX_ENTRY_TTL).contains(&ttl),
        "instance ttl {}",
        ttl
    );
    let code_ttl = e.deployer().get_contract_code_ttl(&axis);
    assert!(code_ttl >= 179 * LPD, "code ttl {}", code_ttl);
}

#[test]
fn test_keepalive_is_a_no_op_above_the_threshold() {
    let (e, _, _, _, _) = setup_test();
    mainnet_ttls(&e);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    // 120 days left: more than the 30-day threshold, nothing to extend
    client.keepalive();
    assert_eq!(instance_ttl(&e, &axis), MIN_PERSISTENT_TTL - 1);
}

#[test]
fn test_keepalive_works_while_frozen() {
    let (e, axis, _, _, _) = aged_contract();
    let client = AxisClient::new(&e, &axis);
    client.freeze(&true);
    client.keepalive();
    assert!(instance_ttl(&e, &axis) >= 179 * LPD);
}

#[test]
fn test_trading_extends_the_contract() {
    let (e, axis, trader, usd, eur) = aged_contract();
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    store_order(&client, &trader, 1000, &usd, &eur, PRECISION);
    assert!(instance_ttl(&e, &axis) >= 179 * LPD);
}
