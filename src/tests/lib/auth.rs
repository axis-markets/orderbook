//! Authorization trees. Makers are paid through the trader's allowance with the contract as
//! spender, so the trader signs the root call only, whatever the book looks like at
//! execution time.
use super::setup::{
    actor, balance, fake_asset, fund, fund_xrf, list_asset, max_live_until, register_axis,
    setup_oracle, store_order, START_TIMESTAMP, UNIT_PRICE,
};
use crate::order::{OrderKind, TradeDirection};
use crate::orderbook::PRECISION;
use crate::trade::{Approval, TradeStep};
use crate::AxisClient;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Vec};

/// Book with one maker selling 1000 USD for EUR at 1, set up under mocked auth; the taker
/// holds 1000 EUR with a standing allowance. Auth mocking is dropped before returning
fn book() -> (Env, Address, Address, Address, Address, Address, u128) {
    let e = Env::default();
    e.ledger().set_timestamp(START_TIMESTAMP);
    e.mock_all_auths();
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    setup_oracle(&e, &issuer);
    list_asset(&e, &usd, UNIT_PRICE);
    list_asset(&e, &eur, UNIT_PRICE);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    let taker = Address::generate(&e);
    fund_xrf(&e, &taker);
    fund(&e, &usd, &axis, &maker, 10000);
    fund(&e, &eur, &axis, &taker, 1000);
    let order = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    e.set_auths(&[]);
    (e, axis, maker, taker, usd, eur, order)
}

#[allow(clippy::too_many_arguments)]
fn trade_args(
    e: &Env,
    taker: &Address,
    amount: i128,
    eur: &Address,
    usd: &Address,
    orders: &Vec<u128>,
    nonce: u64,
    approve: &Option<Approval>,
) -> soroban_sdk::Vec<soroban_sdk::Val> {
    (
        &TradeDirection::Sell,
        &OrderKind::Fill,
        taker,
        &amount,
        eur,
        usd,
        &PRECISION,
        orders,
        &nonce,
        &0u64,
        approve,
    )
        .into_val(e)
}

#[test]
fn test_trade_signs_the_root_call_only() {
    let (e, axis, maker, taker, usd, eur, order) = book();
    let client = AxisClient::new(&e, &axis);
    let orders = Vec::from_array(&e, [order]);
    let approve = None;
    e.mock_auths(&[MockAuth {
        address: &taker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "trade",
            args: trade_args(&e, &taker, 1000, &eur, &usd, &orders, 1, &approve),
            sub_invokes: &[],
        },
    }]);
    let (sold, bought, _) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &taker,
        &1000,
        &eur,
        &usd,
        &PRECISION,
        &orders,
        &1,
        &0,
        &approve,
    );
    assert_eq!((sold, bought), (1000, 1000));
    assert_eq!(balance(&e, &eur, &maker), 1000);
    assert_eq!(balance(&e, &usd, &taker), 1000);
}

#[test]
fn test_trade_with_approval_signs_the_approve_sub_invocation() {
    let (e, axis, _, taker, usd, eur, order) = book();
    let client = AxisClient::new(&e, &axis);
    // the taker holds EUR but granted nothing yet
    e.mock_all_auths();
    super::setup::approve(&e, &eur, &axis, &taker, 0);
    e.set_auths(&[]);
    let orders = Vec::from_array(&e, [order]);
    let live_until = max_live_until(&e);
    let approve = Some(Approval {
        asset: eur.clone(),
        amount: 1000,
        live_until,
    });
    e.mock_auths(&[MockAuth {
        address: &taker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "trade",
            args: trade_args(&e, &taker, 1000, &eur, &usd, &orders, 1, &approve),
            sub_invokes: &[MockAuthInvoke {
                contract: &eur,
                fn_name: "approve",
                args: (&taker, &axis, &1000i128, &live_until).into_val(&e),
                sub_invokes: &[],
            }],
        },
    }]);
    let (sold, bought, _) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &taker,
        &1000,
        &eur,
        &usd,
        &PRECISION,
        &orders,
        &1,
        &0,
        &approve,
    );
    assert_eq!((sold, bought), (1000, 1000));
}

#[test]
fn test_trade_with_unsigned_approval_fails() {
    let (e, axis, _, taker, usd, eur, order) = book();
    let client = AxisClient::new(&e, &axis);
    let orders = Vec::from_array(&e, [order]);
    let approve = Some(Approval {
        asset: eur.clone(),
        amount: 1000,
        live_until: max_live_until(&e),
    });
    // root signed, the approve sub-invocation is not
    e.mock_auths(&[MockAuth {
        address: &taker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "trade",
            args: trade_args(&e, &taker, 1000, &eur, &usd, &orders, 1, &approve),
            sub_invokes: &[],
        },
    }]);
    assert!(client
        .try_trade(
            &TradeDirection::Sell,
            &OrderKind::Fill,
            &taker,
            &1000,
            &eur,
            &usd,
            &PRECISION,
            &orders,
            &1,
            &0,
            &approve,
        )
        .is_err());
}

#[test]
fn test_signed_trade_survives_a_book_change() {
    // The taker signs against a 1000 USD order; before the call lands another taker takes
    // 600 of it. The signed arguments do not depend on the book, so the call still succeeds
    // with the 400 left
    let (e, axis, maker, taker, usd, eur, order) = book();
    let client = AxisClient::new(&e, &axis);
    let orders = Vec::from_array(&e, [order]);
    let approve = None;
    let signed = MockAuthInvoke {
        contract: &axis,
        fn_name: "trade",
        args: trade_args(&e, &taker, 1000, &eur, &usd, &orders, 1, &approve),
        sub_invokes: &[],
    };

    e.mock_all_auths();
    let other = actor(&e);
    fund(&e, &eur, &axis, &other, 600);
    super::setup::trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &other,
        600,
        &eur,
        &usd,
        PRECISION,
        &orders,
    );
    assert_eq!(client.order(&order).unwrap().amount, 400);

    e.mock_auths(&[MockAuth {
        address: &taker,
        invoke: &signed,
    }]);
    let (sold, bought, _) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Fill,
        &taker,
        &1000,
        &eur,
        &usd,
        &PRECISION,
        &orders,
        &1,
        &0,
        &approve,
    );
    assert_eq!((sold, bought), (400, 400));
    assert_eq!(balance(&e, &eur, &maker), 1000);
    assert!(client.order(&order).is_none());
}

#[test]
fn test_crossfill_signs_the_root_call_only() {
    let (e, axis, _, owner, usd, eur, maker_order) = book();
    let client = AxisClient::new(&e, &axis);
    // the taker of the book becomes the owner of an existing EUR order
    e.mock_all_auths();
    let taker_order = store_order(&client, &owner, 1000, &eur, &usd, PRECISION);
    let cranker = Address::generate(&e);
    e.set_auths(&[]);
    let orders = Vec::from_array(&e, [maker_order]);
    e.mock_auths(&[MockAuth {
        address: &cranker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "crossfill",
            args: (&cranker, &taker_order, &orders).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    let (sold, bought, profit) = client.crossfill(&cranker, &taker_order, &orders);
    assert_eq!((sold, bought, profit), (1000, 1000, 0));
}

#[test]
fn test_swap_signs_the_root_call_only() {
    let (e, axis, _, taker, usd, eur, order) = book();
    let client = AxisClient::new(&e, &axis);
    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: usd.clone(),
            orders: Vec::from_array(&e, [order]),
        }],
    );
    let approve: Option<Approval> = None;
    e.mock_auths(&[MockAuth {
        address: &taker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "swap",
            args: (
                &TradeDirection::Sell,
                &taker,
                &eur,
                &1000i128,
                &1000i128,
                &path,
                &approve,
            )
                .into_val(&e),
            sub_invokes: &[],
        },
    }]);
    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &taker,
        &eur,
        &1000,
        &1000,
        &path,
        &approve,
    );
    assert_eq!((sold, bought), (1000, 1000));
    assert_eq!(balance(&e, &usd, &taker), 1000);
}

#[test]
fn test_unsigned_trade_fails() {
    let (e, axis, _, taker, usd, eur, order) = book();
    let client = AxisClient::new(&e, &axis);
    let orders = Vec::from_array(&e, [order]);
    assert!(client
        .try_trade(
            &TradeDirection::Sell,
            &OrderKind::Fill,
            &taker,
            &1000,
            &eur,
            &usd,
            &PRECISION,
            &orders,
            &1,
            &0,
            &None,
        )
        .is_err());
}

#[test]
fn test_update_approvals_sign_the_approve_sub_invocations() {
    let (e, axis, maker, _, usd, _, order) = book();
    let client = AxisClient::new(&e, &axis);
    let live_until = max_live_until(&e);
    let updates = Vec::from_array(&e, [super::setup::order_update(order, 2000, PRECISION)]);
    let approvals = Vec::from_array(
        &e,
        [Approval {
            asset: usd.clone(),
            amount: 2000,
            live_until,
        }],
    );
    // the owner signs `update` and the `approve` it performs on the listed asset
    e.mock_auths(&[MockAuth {
        address: &maker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "update",
            args: (&maker, &updates, &approvals).into_val(&e),
            sub_invokes: &[MockAuthInvoke {
                contract: &usd,
                fn_name: "approve",
                args: (&maker, &axis, &2000i128, &live_until).into_val(&e),
                sub_invokes: &[],
            }],
        },
    }]);
    client.update(&maker, &updates, &approvals);
    assert_eq!(client.order(&order).unwrap().amount, 2000);

    // without the signed sub-invocation the call fails
    e.mock_auths(&[MockAuth {
        address: &maker,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "update",
            args: (&maker, &updates, &approvals).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_update(&maker, &updates, &approvals).is_err());
}
