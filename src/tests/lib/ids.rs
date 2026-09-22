//! Client-chosen order ids derived from the owner and a nonce.
use super::setup::{
    actor, code, ensure_market, fund, no_orders, register_axis, remove_orders, setup_test,
};
use crate::order::{order_id, OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Env};

/// The id derivation, reproduced independently of the contract code
fn expected_id(e: &Env, owner: &Address, nonce: u64) -> u128 {
    let hash = e
        .crypto()
        .sha256(&(owner.clone(), nonce).to_xdr(e))
        .to_array();
    let mut prefix = [0u8; 16];
    prefix.copy_from_slice(&hash[..16]);
    u128::from_be_bytes(prefix)
}

fn create_order_with_nonce(
    client: &AxisClient,
    trader: &Address,
    selling: &Address,
    buying: &Address,
    nonce: u64,
) -> Result<Option<u128>, u32> {
    let e = client.env.clone();
    ensure_market(client, selling, buying);
    match client.try_trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        trader,
        &1000,
        selling,
        buying,
        &PRECISION,
        &no_orders(&e),
        &nonce,
        &0,
        &None,
    ) {
        Ok(Ok((_, _, id))) => Ok(id),
        Ok(Err(err)) => panic!("conversion error {:?}", err),
        res => Err(code(res).unwrap()),
    }
}

#[test]
fn test_order_id_is_deterministic() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);

    let id = order_id(&e, &trader, 12345);
    assert_eq!(id, expected_id(&e, &trader, 12345));
    let created = create_order_with_nonce(&client, &trader, &usd, &eur, 12345).unwrap();
    assert_eq!(created, Some(id));
    assert_eq!(client.order(&id).unwrap().id, id);
}

#[test]
fn test_ids_differ_by_owner_and_nonce() {
    let (e, trader, _, _, _) = setup_test();
    let other = actor(&e);
    assert_ne!(order_id(&e, &trader, 1), order_id(&e, &trader, 2));
    assert_ne!(order_id(&e, &trader, 1), order_id(&e, &other, 1));
}

#[test]
fn test_duplicate_nonce_is_rejected() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);

    let id = create_order_with_nonce(&client, &trader, &usd, &eur, 5)
        .unwrap()
        .unwrap();
    assert_eq!(
        create_order_with_nonce(&client, &trader, &usd, &eur, 5),
        Err(711)
    );
    // the same nonce on another pair collides as well: ids do not depend on the market
    assert_eq!(
        create_order_with_nonce(&client, &trader, &eur, &usd, 5),
        Err(711)
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}

#[test]
fn test_nonce_is_reusable_once_the_order_is_gone() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);

    let id = create_order_with_nonce(&client, &trader, &usd, &eur, 5)
        .unwrap()
        .unwrap();
    remove_orders(&client, &trader, &[id]);
    let again = create_order_with_nonce(&client, &trader, &usd, &eur, 5)
        .unwrap()
        .unwrap();
    assert_eq!(again, id);
}

#[test]
fn test_nonce_is_ignored_when_nothing_stored() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 10000);
    let id = create_order_with_nonce(&client, &trader, &usd, &eur, 5)
        .unwrap()
        .unwrap();

    // a Fill with a colliding nonce is fine: no order is created
    let (sold, bought, created) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &trader,
        &1000,
        &usd,
        &eur,
        &PRECISION,
        &no_orders(&e),
        &5,
        &0,
        &None,
    );
    assert_eq!((sold, bought, created), (0, 0, None));
    assert_eq!(client.order(&id).unwrap().amount, 1000);
}
