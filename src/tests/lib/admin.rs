//! Safety admin controls: the emergency switch, the role handover and the minimum trade size.
extern crate std;
use super::mock_oracle::MockOracleClient;
use super::setup::{
    actor, code, config, fund, no_orders, open_market, oracle_address, order_update, register_axis,
    register_oracle, safety_admin, setup_test, store_order, try_remove_orders, try_trade,
    try_update_one, MARKET_LISTING_FEE, MIN_TRADE_SIZE, ORACLE_DAILY_FEE, ORACLE_DECIMALS,
};
use crate::events::{DelegateEvent, FreezeEvent, OracleEvent};
use crate::market::DataKey;
use crate::order::{OrderKind, TradeDirection};
use crate::orderbook::PRECISION;
use crate::trade::TradeStep;
use crate::{Axis, AxisClient};
use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Event, IntoVal, Vec};

const AMOUNT: i128 = 1_000_000;

/// Create a sell limit order for `amount` of `selling`, returning its id
fn creat_sell_order(
    client: &AxisClient,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
) -> u128 {
    let e = client.env.clone();
    fund(&e, selling, &client.address, trader, amount);
    store_order(client, trader, amount, selling, buying, PRECISION)
}

/// Attempt a sell limit trade, returning the contract error code on failure
fn try_sell(
    client: &AxisClient,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
) -> Option<u32> {
    let e = client.env.clone();
    fund(&e, selling, &client.address, trader, amount);
    try_trade(
        client,
        TradeDirection::Sell,
        OrderKind::Limit,
        trader,
        amount,
        selling,
        buying,
        PRECISION,
        &no_orders(&e),
    )
}

#[test]
fn test_constructor_stores_safety_settings() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let config = client.config();
    assert_eq!(config.safety_admin, safety_admin(&e));
    assert_eq!(config.min_trade_size, MIN_TRADE_SIZE);
    //a fresh contract trades normally
    assert_eq!(client.frozen(), false);
}

#[test]
#[should_panic(expected = "#706")]
fn test_constructor_rejects_negative_min_trade_size() {
    let (e, _, _, _, _) = setup_test();
    e.register(Axis, (safety_admin(&e), oracle_address(&e), -1i128));
}

#[test]
fn test_freeze_blocks_trade() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    //the market already exists, so the rejection cannot come from market creation
    creat_sell_order(&client, &trader, AMOUNT, &usd, &eur);

    client.freeze(&true);
    assert_eq!(client.frozen(), true);
    assert_eq!(try_sell(&client, &trader, AMOUNT, &usd, &eur), Some(730));
}

#[test]
fn test_freeze_blocks_fill_and_fill_or_kill() {
    let (e, maker, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let taker = actor(&e);
    let created = creat_sell_order(&client, &maker, AMOUNT, &usd, &eur);
    let orders = Vec::from_array(&e, [created]);
    fund(&e, &eur, &client.address, &taker, AMOUNT);

    client.freeze(&true);
    for kind in [OrderKind::Fill, OrderKind::FillOrKill] {
        assert_eq!(
            try_trade(
                &client,
                TradeDirection::Sell,
                kind,
                &taker,
                AMOUNT,
                &eur,
                &usd,
                PRECISION,
                &orders,
            ),
            Some(730)
        );
    }
}

#[test]
fn test_freeze_blocks_crossfill() {
    let (e, maker, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let taker_owner = actor(&e);
    let cranker = actor(&e);
    let maker_order = creat_sell_order(&client, &maker, AMOUNT, &usd, &eur);
    let taker_order = creat_sell_order(&client, &taker_owner, AMOUNT, &eur, &usd);

    client.freeze(&true);
    assert_eq!(
        code(client.try_crossfill(&cranker, &taker_order, &Vec::from_array(&e, [maker_order]))),
        Some(730)
    );
    //both orders are untouched
    assert_eq!(client.order(&maker_order).unwrap().amount, AMOUNT);
    assert_eq!(client.order(&taker_order).unwrap().amount, AMOUNT);
}

#[test]
fn test_freeze_blocks_swap() {
    let (e, maker, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let trader = actor(&e);
    let created = creat_sell_order(&client, &maker, AMOUNT, &eur, &usd);
    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: eur.clone(),
            orders: Vec::from_array(&e, [created]),
        }],
    );
    fund(&e, &usd, &client.address, &trader, AMOUNT);

    client.freeze(&true);
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &AMOUNT,
            &AMOUNT,
            &path,
            &None
        )),
        Some(730)
    );
}

#[test]
fn test_freeze_blocks_subsidize() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    creat_sell_order(&client, &trader, AMOUNT, &usd, &eur);

    client.freeze(&true);
    assert_eq!(
        code(client.try_subsidize(&trader, &usd, &eur, &MARKET_LISTING_FEE)),
        Some(730)
    );
}

#[test]
fn test_freeze_blocks_update_and_removal() {
    //the contract holds no funds, so a full stop strands nothing: order management is
    //blocked along with trading
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let order = creat_sell_order(&client, &trader, AMOUNT, &usd, &eur);

    client.freeze(&true);
    assert_eq!(try_remove_orders(&client, &trader, &[order]), Some(730));
    assert_eq!(
        try_update_one(&client, &trader, order_update(order, AMOUNT / 2, PRECISION)),
        Some(730)
    );
    assert_eq!(client.order(&order).unwrap().amount, AMOUNT);
}

#[test]
fn test_frozen_contract_still_answers_views() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let order = creat_sell_order(&client, &trader, AMOUNT, &usd, &eur);

    client.freeze(&true);
    assert_eq!(client.order(&order).unwrap().amount, AMOUNT);
    assert!(client.market(&usd, &eur).is_some());
    assert_eq!(client.config(), config(&e));
}

#[test]
fn test_unfreeze_resumes_trading() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    creat_sell_order(&client, &trader, AMOUNT, &usd, &eur);

    client.freeze(&true);
    assert_eq!(try_sell(&client, &trader, AMOUNT, &usd, &eur), Some(730));

    client.freeze(&false);
    assert_eq!(client.frozen(), false);
    //the very trade that was rejected now goes through
    assert_eq!(try_sell(&client, &trader, AMOUNT, &usd, &eur), None);
}

#[test]
fn test_freeze_is_idempotent() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));

    //releasing an already open contract changes nothing
    client.freeze(&false);
    assert_eq!(client.frozen(), false);
    creat_sell_order(&client, &trader, AMOUNT, &usd, &eur);

    client.freeze(&true);
    client.freeze(&true);
    assert_eq!(client.frozen(), true);
    assert_eq!(try_sell(&client, &trader, AMOUNT, &usd, &eur), Some(730));
}

#[test]
fn test_freeze_emits_event() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);

    //the host keeps the events of the latest top-level invocation, so check each flip in turn
    client.freeze(&true);
    let engaged = FreezeEvent { frozen: true };
    assert_eq!(
        e.events().all().filter_by_contract(&axis),
        [engaged.to_xdr(&e, &axis)]
    );

    client.freeze(&false);
    let released = FreezeEvent { frozen: false };
    assert_eq!(
        e.events().all().filter_by_contract(&axis),
        [released.to_xdr(&e, &axis)]
    );
}

#[test]
fn test_freeze_requires_safety_admin() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    open_market(&client, &usd, &eur);

    //drop the blanket auth mock: nobody authorizes the call now
    e.set_auths(&[]);
    assert!(
        client.try_freeze(&true).is_err(),
        "freeze must fail without safety admin authorization"
    );

    e.mock_all_auths();
    assert_eq!(client.frozen(), false);
    assert_eq!(try_sell(&client, &trader, AMOUNT, &usd, &eur), None);
}

#[test]
fn test_freeze_rejects_other_authorizer() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let intruder = Address::generate(&e);

    //the intruder authorizes the call, but the contract asks the safety admin
    e.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "freeze",
            args: (true,).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(
        client.try_freeze(&true).is_err(),
        "only the safety admin may operate the switch"
    );
}

#[test]
fn test_delegate_transfers_the_role() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let successor = Address::generate(&e);

    client.delegate(&successor);
    assert_eq!(client.config().safety_admin, successor);

    //the successor operates the switch
    e.mock_auths(&[MockAuth {
        address: &successor,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "freeze",
            args: (true,).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    client.freeze(&true);
    assert_eq!(client.frozen(), true);
}

#[test]
fn test_delegate_revokes_the_previous_admin() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    //hand the role over twice, so the revoked admin is an address the auth mock accepts
    let first = Address::generate(&e);
    let second = Address::generate(&e);
    client.delegate(&first);
    client.delegate(&second);
    assert_eq!(client.config().safety_admin, second);

    //the former admin authorizes the call, but the contract now asks the current one
    e.mock_auths(&[MockAuth {
        address: &first,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "freeze",
            args: (true,).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(
        client.try_freeze(&true).is_err(),
        "the previous admin must lose the role"
    );
    assert_eq!(client.frozen(), false);
}

#[test]
fn test_delegate_requires_current_safety_admin() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let intruder = Address::generate(&e);

    //the intruder tries to grant themselves the role
    e.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "delegate",
            args: (&intruder,).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(
        client.try_delegate(&intruder).is_err(),
        "only the current safety admin may hand the role over"
    );

    e.mock_all_auths();
    assert_eq!(client.config().safety_admin, safety_admin(&e));
}

#[test]
fn test_delegate_works_while_frozen() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let successor = Address::generate(&e);

    client.freeze(&true);
    //handing the role over must stay possible during an incident
    client.delegate(&successor);
    assert_eq!(client.config().safety_admin, successor);
    //and the contract stays frozen through the handover
    assert_eq!(client.frozen(), true);
}

#[test]
fn test_delegate_emits_event() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let successor = Address::generate(&e);

    client.delegate(&successor);
    let expected = DelegateEvent {
        admin: successor,
        previous: safety_admin(&e),
    };
    assert_eq!(
        e.events().all().filter_by_contract(&axis),
        [expected.to_xdr(&e, &axis)]
    );
}

#[test]
fn test_set_floor_updates_config() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    //10 USD in 7-decimal fixed point
    let size = 10_0000000i128;

    client.set_floor(&size);
    assert_eq!(client.config().min_trade_size, size);
    //the rest of the configuration is untouched
    assert_eq!(client.config().oracle, oracle_address(&e));
    assert_eq!(client.config().market_listing_fee, MARKET_LISTING_FEE);

    //zero disables the limit
    client.set_floor(&0);
    assert_eq!(client.config().min_trade_size, 0);
}

#[test]
fn test_set_floor_rejects_negative() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));

    assert_eq!(code(client.try_set_floor(&-1)), Some(706));
    assert_eq!(client.config().min_trade_size, MIN_TRADE_SIZE);
}

#[test]
fn test_set_floor_requires_safety_admin() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));

    e.set_auths(&[]);
    assert!(
        client.try_set_floor(&5_0000000).is_err(),
        "set_floor must fail without safety admin authorization"
    );

    e.mock_all_auths();
    assert_eq!(client.config().min_trade_size, MIN_TRADE_SIZE);
}

#[test]
fn test_set_oracle_updates_config_and_decimals() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let successor = register_oracle(&e, 18);

    client.set_oracle(&successor);
    assert_eq!(client.config().oracle, successor);
    //the rest of the configuration is untouched, the listing fee follows the same daily fee
    assert_eq!(client.config().safety_admin, safety_admin(&e));
    assert_eq!(client.config().market_listing_fee, MARKET_LISTING_FEE);
    //the price decimals snapshot follows the new oracle
    e.as_contract(&axis, || {
        let decimals: u32 = e
            .storage()
            .instance()
            .get(&DataKey::OracleDecimals)
            .unwrap();
        assert_eq!(decimals, 18);
    });
}

#[test]
fn test_set_oracle_re_derives_the_listing_fee() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    //the successor charges 2 XRF a day per asset: new markets cost 90 days of that
    let successor = register_oracle(&e, ORACLE_DECIMALS);
    MockOracleClient::new(&e, &successor).set_daily_fee(&(2 * ORACLE_DAILY_FEE));

    client.set_oracle(&successor);
    assert_eq!(
        client.config().market_listing_fee,
        2 * ORACLE_DAILY_FEE * 90
    );
}

#[test]
fn test_set_oracle_rejects_oracle_without_fee_config() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let successor = register_oracle(&e, ORACLE_DECIMALS);
    MockOracleClient::new(&e, &successor).set_daily_fee(&0);

    assert_eq!(code(client.try_set_oracle(&successor)), Some(723));
    //nothing changed
    assert_eq!(client.config().oracle, oracle_address(&e));
    assert_eq!(client.config().market_listing_fee, MARKET_LISTING_FEE);
}

#[test]
fn test_set_oracle_rejects_too_many_price_decimals() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));

    //too many price decimals
    assert_eq!(
        code(client.try_set_oracle(&register_oracle(&e, 31))),
        Some(723)
    );

    assert_eq!(client.config().oracle, oracle_address(&e));
}

#[test]
fn test_set_oracle_requires_safety_admin() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let successor = register_oracle(&e, ORACLE_DECIMALS);
    let intruder = Address::generate(&e);

    e.set_auths(&[]);
    assert!(
        client.try_set_oracle(&successor).is_err(),
        "set_oracle must fail without safety admin authorization"
    );

    //the intruder authorizes the call, but the contract asks the safety admin
    e.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &axis,
            fn_name: "set_oracle",
            args: (successor.clone(),).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(
        client.try_set_oracle(&successor).is_err(),
        "only the safety admin may replace the oracle"
    );

    e.mock_all_auths();
    assert_eq!(client.config().oracle, oracle_address(&e));
}

#[test]
fn test_set_oracle_works_while_frozen() {
    let (e, _, _, _, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    let successor = register_oracle(&e, ORACLE_DECIMALS);

    client.freeze(&true);
    client.set_oracle(&successor);
    assert_eq!(client.config().oracle, successor);
    assert_eq!(client.frozen(), true);
}

#[test]
fn test_set_oracle_emits_event() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let successor = register_oracle(&e, ORACLE_DECIMALS);

    client.set_oracle(&successor);
    let handover = OracleEvent {
        oracle: successor,
        previous: oracle_address(&e),
    };
    assert_eq!(
        e.events().all().filter_by_contract(&axis),
        [handover.to_xdr(&e, &axis)]
    );
}
