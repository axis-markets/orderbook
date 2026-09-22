//! Market creation, oracle verification and provisioning.
extern crate std;
use super::mock_oracle::MockOracle;
use super::setup::{
    actor, advance, balance, code, fake_asset, fund, fund_xrf, list_asset, no_orders,
    oracle_address, oracle_client, register_axis, remove_orders, safety_admin, setup_oracle,
    setup_test, store_order, trade, try_trade, xrf, MARKET_LISTING_FEE, MIN_TRADE_SIZE,
    ORACLE_DAILY_FEE, ORACLE_DECIMALS, START_TIMESTAMP, UNIT_PRICE,
};
use crate::market::{canonical, DataKey};
use crate::order::{OrderKind, TradeDirection};
use crate::orderbook::PRECISION;
use crate::reflector_beam::Asset;
use crate::trade::TradeStep;
use crate::{Axis, AxisClient};
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token::StellarAssetClient, Address, Env, IntoVal, Vec};

/// Stores a sell limit order for `amount` of `selling` at price 1
fn create_sell_order(
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

fn xrf_balance(e: &Env, account: &Address) -> i128 {
    balance(e, &xrf(e), account)
}

fn tracked_until(e: &Env, consumer: &Address, asset: &Address) -> u64 {
    oracle_client(e)
        .tracked_until(
            consumer,
            &Vec::from_array(e, [Asset::Stellar(asset.clone())]),
        )
        .get(0)
        .unwrap()
}

#[test]
fn test_constructor_stores_config() {
    let (e, _, _, _, _) = setup_test();
    let axis = register_axis(&e);
    let config = AxisClient::new(&e, &axis).config();
    assert_eq!(config.oracle, oracle_address(&e));
    assert_eq!(config.market_listing_fee, MARKET_LISTING_FEE);
    assert_eq!(config.safety_admin, safety_admin(&e));
    assert_eq!(config.min_trade_size, MIN_TRADE_SIZE);
    e.as_contract(&axis, || {
        let decimals: u32 = e
            .storage()
            .instance()
            .get(&DataKey::OracleDecimals)
            .unwrap();
        assert_eq!(decimals, ORACLE_DECIMALS);
    });
}

#[test]
#[should_panic(expected = "#723")]
fn test_constructor_rejects_oracle_with_too_many_decimals() {
    let (e, _, _, _, _) = setup_test();
    let oracle = e.register(MockOracle, (xrf(&e), 31u32, ORACLE_DAILY_FEE));
    e.register(Axis, (safety_admin(&e), oracle, MIN_TRADE_SIZE));
}

#[test]
fn test_constructor_derives_the_listing_fee_from_the_oracle() {
    let (e, _, _, _, _) = setup_test();
    // 90 days of the oracle's daily per-asset fee
    let oracle = e.register(MockOracle, (xrf(&e), ORACLE_DECIMALS, 3_0000000i128));
    let axis = e.register(Axis, (safety_admin(&e), oracle, MIN_TRADE_SIZE));
    let config = AxisClient::new(&e, &axis).config();
    assert_eq!(config.market_listing_fee, 3_0000000 * 90);
}

#[test]
#[should_panic(expected = "#723")]
fn test_constructor_rejects_oracle_without_fee_config() {
    let (e, _, _, _, _) = setup_test();
    let oracle = e.register(MockOracle, (xrf(&e), ORACLE_DECIMALS, 0i128));
    e.register(Axis, (safety_admin(&e), oracle, MIN_TRADE_SIZE));
}

#[test]
#[should_panic(expected = "#723")]
fn test_constructor_rejects_negative_daily_fee() {
    let (e, _, _, _, _) = setup_test();
    let oracle = e.register(MockOracle, (xrf(&e), ORACLE_DECIMALS, -1i128));
    e.register(Axis, (safety_admin(&e), oracle, MIN_TRADE_SIZE));
}

#[test]
#[should_panic(expected = "#723")]
fn test_constructor_rejects_a_listing_fee_overflow() {
    let (e, _, _, _, _) = setup_test();
    let oracle = e.register(MockOracle, (xrf(&e), ORACLE_DECIMALS, i128::MAX / 89));
    e.register(Axis, (safety_admin(&e), oracle, MIN_TRADE_SIZE));
}

#[test]
fn test_subsidize_opens_a_market() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let xrf_before = xrf_balance(&e, &trader);
    let subsidy = 2 * ORACLE_DAILY_FEE;
    let amount = MARKET_LISTING_FEE + subsidy;

    let ttls = client.subsidize(&trader, &usd, &eur, &amount);

    // market record for the pair, both sides quoted by the oracle
    let market = client.market(&usd, &eur).unwrap();
    let (a, b) = canonical(&usd, &eur);
    assert_eq!(market.a.asset, a);
    assert_eq!(market.b.asset, b);
    assert!(market.a.listed && market.b.listed);
    assert_eq!(market.a.decimals, 7);
    assert_eq!(market.b.decimals, 7);
    assert_eq!(market.created, START_TIMESTAMP);

    // exactly `amount` was burned: the listing fee, then the rest as a subsidy
    assert_eq!(xrf_balance(&e, &trader), xrf_before - amount);
    let days = amount / 2 / ORACLE_DAILY_FEE;
    let expected = START_TIMESTAMP + days as u64 * 86_400;
    assert_eq!(ttls, Vec::from_array(&e, [expected, expected]));
    assert_eq!(tracked_until(&e, &axis, &usd), expected);
    assert_eq!(tracked_until(&e, &axis, &eur), expected);

    // the open market takes limit orders
    let id = create_sell_order(&client, &trader, 1000, &usd, &eur);
    assert!(client.order(&id).is_some());
}

#[test]
fn test_subsidize_below_the_listing_fee_opens_nothing() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let xrf_before = xrf_balance(&e, &trader);

    assert_eq!(
        code(client.try_subsidize(&trader, &usd, &eur, &(MARKET_LISTING_FEE - 1))),
        Some(706)
    );
    assert!(client.market(&usd, &eur).is_none());
    assert_eq!(xrf_balance(&e, &trader), xrf_before);
}

#[test]
fn test_subsidize_with_exactly_the_listing_fee() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let xrf_before = xrf_balance(&e, &trader);

    // the fee alone opens the market: 90 days split across the two listed assets, no subsidy
    let ttls = client.subsidize(&trader, &usd, &eur, &MARKET_LISTING_FEE);
    assert_eq!(xrf_balance(&e, &trader), xrf_before - MARKET_LISTING_FEE);
    let expected = START_TIMESTAMP + 45 * 86_400;
    assert_eq!(ttls, Vec::from_array(&e, [expected, expected]));
    assert_eq!(tracked_until(&e, &axis, &usd), expected);

    // on the open market any positive amount extends access, the fee is not charged again
    let ttls = client.subsidize(&trader, &usd, &eur, &(2 * ORACLE_DAILY_FEE));
    assert_eq!(ttls.get(0).unwrap(), expected + 86_400);
    assert_eq!(
        xrf_balance(&e, &trader),
        xrf_before - MARKET_LISTING_FEE - 2 * ORACLE_DAILY_FEE
    );
}

#[test]
fn test_limit_order_requires_an_open_market() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &usd, &axis, &trader, 1000);
    let xrf_before = xrf_balance(&e, &trader);

    // both assets are quoted, but nobody opened the market
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION,
            &no_orders(&e),
        ),
        Some(721)
    );
    assert!(client.market(&usd, &eur).is_none());
    assert_eq!(xrf_balance(&e, &trader), xrf_before);
}

#[test]
fn test_market_shared_by_both_orientations() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);
    let market = client.market(&usd, &eur).unwrap();
    let xrf_after_first = xrf_balance(&e, &trader);
    let access = tracked_until(&e, &axis, &usd);

    // the opposite direction uses the same market: no second fee, no new access
    create_sell_order(&client, &trader, 1000, &eur, &usd);
    assert_eq!(client.market(&eur, &usd).unwrap(), market);
    assert_eq!(xrf_balance(&e, &trader), xrf_after_first);
    assert_eq!(tracked_until(&e, &axis, &usd), access);
}

#[test]
fn test_market_without_listed_assets_rejected() {
    let (e, trader, issuer, _, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let chf = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &gbp, &axis, &trader, 1000);
    let xrf_before = xrf_balance(&e, &trader);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &trader,
            1000,
            &gbp,
            &chf,
            PRECISION,
            &no_orders(&e),
        ),
        Some(721)
    );
    assert_eq!(
        code(client.try_subsidize(&trader, &gbp, &chf, &MARKET_LISTING_FEE)),
        Some(721)
    );
    assert!(client.market(&gbp, &chf).is_none());
    assert_eq!(xrf_balance(&e, &trader), xrf_before);
}

#[test]
fn test_market_with_only_selling_asset_listed() {
    let (e, trader, issuer, usd, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let xrf_before = xrf_balance(&e, &trader);

    let amount = MARKET_LISTING_FEE + ORACLE_DAILY_FEE;
    client.subsidize(&trader, &usd, &gbp, &amount);

    let market = client.market(&usd, &gbp).unwrap();
    let usd_side = market.side(&usd);
    let gbp_side = market.side(&gbp);
    assert_eq!(usd_side.asset, usd);
    assert!(usd_side.listed);
    assert_eq!(usd_side.decimals, 7);
    assert_eq!(gbp_side.asset, gbp);
    assert!(!gbp_side.listed);
    assert_eq!(gbp_side.decimals, 0);
    // the whole amount provisions the single listed asset: 90 days for the fee, one more
    assert_eq!(xrf_balance(&e, &trader), xrf_before - amount);
    let days = amount / ORACLE_DAILY_FEE;
    assert_eq!(
        tracked_until(&e, &axis, &usd),
        START_TIMESTAMP + days as u64 * 86_400
    );
    assert_eq!(tracked_until(&e, &axis, &gbp), 0);
}

#[test]
fn test_market_with_only_buying_asset_listed() {
    let (e, trader, issuer, usd, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);

    // selling the unlisted asset: the order is valued on the USD it buys
    create_sell_order(&client, &trader, 1000, &gbp, &usd);

    let market = client.market(&gbp, &usd).unwrap();
    assert!(market.side(&usd).listed);
    assert!(!market.side(&gbp).listed);
    assert_eq!(client.market(&usd, &gbp).unwrap(), market);
}

#[test]
fn test_taker_trades_do_not_open_markets() {
    let (e, trader, issuer, _, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let chf = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    fund(&e, &gbp, &axis, &trader, 1000);
    let xrf_before = xrf_balance(&e, &trader);

    // nothing to match: a Fill is a no-op, a FillOrKill fails
    let result = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &trader,
        1000,
        &gbp,
        &chf,
        PRECISION,
        &no_orders(&e),
    );
    assert_eq!(result, (0, 0, None));
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::FillOrKill,
            &trader,
            1000,
            &gbp,
            &chf,
            PRECISION,
            &no_orders(&e),
        ),
        Some(709)
    );
    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: chf.clone(),
            orders: Vec::new(&e),
        }],
    );
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &gbp,
            &1000,
            &1,
            &path,
            &None
        )),
        Some(709)
    );

    assert!(client.market(&gbp, &chf).is_none());
    assert_eq!(xrf_balance(&e, &trader), xrf_before);
}

#[test]
#[should_panic(expected = "#10")]
fn test_market_creation_requires_xrf_balance() {
    let (e, _, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    // a sponsor without XRF cannot pay the listing fee
    let poor = Address::generate(&e);
    client.subsidize(&poor, &usd, &eur, &MARKET_LISTING_FEE);
}

#[test]
fn test_subsidize_extends_access() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);
    let access = tracked_until(&e, &axis, &usd);

    // anyone can top up the market feeds: 2 XRF over 2 assets buys one more day each
    let sponsor = actor(&e);
    let xrf_before = xrf_balance(&e, &sponsor);
    let amount = ORACLE_DAILY_FEE * 2;
    let ttls = client.subsidize(&sponsor, &eur, &usd, &amount);

    assert_eq!(ttls.len(), 2);
    assert_eq!(ttls.get(0).unwrap(), access + 86_400);
    assert_eq!(tracked_until(&e, &axis, &usd), access + 86_400);
    assert_eq!(tracked_until(&e, &axis, &eur), access + 86_400);
    assert_eq!(xrf_balance(&e, &sponsor), xrf_before - amount);
}

#[test]
#[should_panic(expected = "#706")]
fn test_subsidize_invalid_amount() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);
    client.subsidize(&trader, &usd, &eur, &0);
}

#[test]
fn test_trade_rejects_prices_out_of_range() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    for price in [0, -1, PRECISION * PRECISION + 1] {
        assert_eq!(
            try_trade(
                &client,
                TradeDirection::Sell,
                OrderKind::Fill,
                &trader,
                1000,
                &usd,
                &eur,
                price,
                &no_orders(&e),
            ),
            Some(705)
        );
    }
}

#[test]
#[should_panic(expected = "#706")]
fn test_trade_rejects_zero_amount() {
    let (e, trader, _, usd, eur) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        0,
        &usd,
        &eur,
        PRECISION,
        &no_orders(&e),
    );
}

#[test]
#[should_panic(expected = "#704")]
fn test_trade_rejects_same_asset() {
    let (e, trader, _, usd, _) = setup_test();
    let client = AxisClient::new(&e, &register_axis(&e));
    trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &trader,
        1000,
        &usd,
        &usd,
        PRECISION,
        &no_orders(&e),
    );
}

/// Environment without mocked auth: the trader holds XRF, 1000 USD and a standing allowance;
/// every call below signs its own auth tree.
fn unmocked_env() -> (Env, Address, Address, Address, Address, Address) {
    let e = Env::default();
    e.ledger().set_timestamp(START_TIMESTAMP);
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    let xrf = setup_oracle(&e, &issuer);
    list_asset(&e, &usd, UNIT_PRICE);
    list_asset(&e, &eur, UNIT_PRICE);
    fund_xrf(&e, &trader);
    let axis = register_axis(&e);
    e.mock_all_auths();
    StellarAssetClient::new(&e, &usd).mint(&trader, &1000);
    // the standing allowance is granted outside the trade
    soroban_sdk::token::Client::new(&e, &usd).approve(
        &trader,
        &axis,
        &1000,
        &super::setup::max_live_until(&e),
    );
    e.set_auths(&[]);
    (e, trader, axis, xrf, usd, eur)
}

/// Open the USD/EUR market by subsidizing it with an explicit auth tree; `with_track` includes
/// the oracle provisioning sub-invocations in the signed tree: one `track` -> `burn` for the
/// market listing fee and one for the rest of `amount`
fn subsidize_with_auth(
    e: &Env,
    sponsor: &Address,
    axis: &Address,
    xrf: &Address,
    usd: &Address,
    eur: &Address,
    with_track: bool,
) -> Vec<u64> {
    let client = AxisClient::new(e, axis);
    let oracle = oracle_address(e);
    let (a, b) = canonical(usd, eur);
    let assets = Vec::from_array(e, [Asset::Stellar(a), Asset::Stellar(b)]);
    let subsidy = ORACLE_DAILY_FEE * 2;
    let amount = MARKET_LISTING_FEE + subsidy;
    let track_args = |fee: &i128| (sponsor, axis, &assets, fee).into_val(e);
    let burn_args = |fee: &i128| (sponsor, fee).into_val(e);
    let listing_burn = [MockAuthInvoke {
        contract: xrf,
        fn_name: "burn",
        args: burn_args(&MARKET_LISTING_FEE),
        sub_invokes: &[],
    }];
    let subsidy_burn = [MockAuthInvoke {
        contract: xrf,
        fn_name: "burn",
        args: burn_args(&subsidy),
        sub_invokes: &[],
    }];
    let tracks = [
        MockAuthInvoke {
            contract: &oracle,
            fn_name: "track",
            args: track_args(&MARKET_LISTING_FEE),
            sub_invokes: &listing_burn,
        },
        MockAuthInvoke {
            contract: &oracle,
            fn_name: "track",
            args: track_args(&subsidy),
            sub_invokes: &subsidy_burn,
        },
    ];
    let sub_invokes: &[MockAuthInvoke] = if with_track { &tracks } else { &[] };
    e.mock_auths(&[MockAuth {
        address: sponsor,
        invoke: &MockAuthInvoke {
            contract: axis,
            fn_name: "subsidize",
            args: (sponsor, usd, eur, &amount).into_val(e),
            sub_invokes,
        },
    }]);
    client.subsidize(sponsor, usd, eur, &amount)
}

/// Sell 1000 USD for EUR on an open market with an auth tree holding the root call only: the
/// order is stored against the standing allowance, nothing below the root is signed
fn trade_with_auth(
    e: &Env,
    trader: &Address,
    axis: &Address,
    usd: &Address,
    eur: &Address,
) -> Option<u128> {
    let client = AxisClient::new(e, axis);
    let amount = 1000i128;
    let nonce = 42u64;
    let empty: Vec<u128> = Vec::new(e);
    let approve: Option<crate::trade::Approval> = None;
    e.mock_auths(&[MockAuth {
        address: trader,
        invoke: &MockAuthInvoke {
            contract: axis,
            fn_name: "trade",
            args: (
                &TradeDirection::Sell,
                &OrderKind::Limit,
                trader,
                &amount,
                usd,
                eur,
                &PRECISION,
                &empty,
                &nonce,
                &0u64,
                &approve,
            )
                .into_val(e),
            sub_invokes: &[],
        },
    }]);
    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        trader,
        &amount,
        usd,
        eur,
        &PRECISION,
        &empty,
        &nonce,
        &0,
        &approve,
    );
    id
}

#[test]
fn test_market_creation_auth_tree() {
    let (e, trader, axis, xrf, usd, eur) = unmocked_env();
    let ttls = subsidize_with_auth(&e, &trader, &axis, &xrf, &usd, &eur, true);
    assert_eq!(ttls.len(), 2);
    let client = AxisClient::new(&e, &axis);
    assert!(client.market(&usd, &eur).is_some());
    // creating an order on the open market signs no provisioning at all
    let id = trade_with_auth(&e, &trader, &axis, &usd, &eur);
    assert!(client.order(&id.unwrap()).is_some());
}

#[test]
#[should_panic(expected = "Auth, InvalidAction")]
fn test_market_creation_requires_track_authorization() {
    let (e, trader, axis, xrf, usd, eur) = unmocked_env();
    subsidize_with_auth(&e, &trader, &axis, &xrf, &usd, &eur, false);
}

/// Drop `asset` from the oracle asset list
fn delist(e: &Env, asset: &Address) {
    oracle_client(e).remove_asset(&Asset::Stellar(asset.clone()));
}

#[test]
fn test_requote_picks_up_a_newly_listed_asset() {
    let (e, trader, issuer, usd, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);

    //the market opens while GBP is not quoted by the oracle
    create_sell_order(&client, &trader, 1000, &usd, &gbp);
    let before = client.market(&usd, &gbp).unwrap();
    assert!(!before.side(&gbp).listed);
    assert_eq!(before.side(&gbp).decimals, 0);
    assert_eq!(before.checked, START_TIMESTAMP);

    //the oracle starts quoting GBP
    list_asset(&e, &gbp, UNIT_PRICE);
    advance(&e, 60);
    let after = client.requote(&usd, &gbp).unwrap();

    assert!(after.side(&gbp).listed);
    assert_eq!(after.side(&gbp).decimals, 7);
    assert!(after.side(&usd).listed);
    assert_eq!(after.checked, START_TIMESTAMP + 60);
    //the creation timestamp is not touched, and the record is what the view reports
    assert_eq!(after.created, START_TIMESTAMP);
    assert_eq!(client.market(&usd, &gbp).unwrap(), after);
}

#[test]
fn test_requote_drops_a_delisted_asset() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);

    delist(&e, &eur);
    let after = client.requote(&usd, &eur).unwrap();

    assert!(after.side(&usd).listed);
    assert!(!after.side(&eur).listed);
    assert_eq!(after.side(&eur).decimals, 0);
}

#[test]
fn test_requote_records_a_fully_delisted_pair() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let id = create_sell_order(&client, &trader, 1000, &usd, &eur);

    delist(&e, &usd);
    delist(&e, &eur);
    let after = client.requote(&usd, &eur).unwrap();
    assert!(!after.side(&usd).listed && !after.side(&eur).listed);

    //no new limit order can be valued on that pair any more
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &trader,
            1000,
            &usd,
            &eur,
            PRECISION,
            &no_orders(&e),
        ),
        Some(721)
    );
    //the order is untouched and still removable
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    remove_orders(&client, &trader, &[id]);
    assert!(client.order(&id).is_none());
}

#[test]
fn test_requote_unknown_pair() {
    let (e, _, issuer, usd, _) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let client = AxisClient::new(&e, &register_axis(&e));

    assert_eq!(client.requote(&usd, &gbp), None);
}

#[test]
fn test_requote_is_permissionless() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);
    delist(&e, &eur);

    //nobody signs for it
    e.set_auths(&[]);
    let after = client.requote(&usd, &eur).unwrap();
    assert!(!after.side(&eur).listed);
    e.mock_all_auths();
}

#[test]
fn test_requote_is_blocked_while_frozen() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);
    delist(&e, &eur);
    client.freeze(&true);

    assert_eq!(code(client.try_requote(&usd, &eur)), Some(730));
    //the record keeps the listing it had
    assert!(client.market(&usd, &eur).unwrap().side(&eur).listed);
}

#[test]
fn test_subsidize_follows_the_current_listing() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);
    let usd_access = tracked_until(&e, &axis, &usd);
    let eur_access = tracked_until(&e, &axis, &eur);

    //EUR stops being quoted: the whole fee must go to the surviving asset
    delist(&e, &eur);
    let sponsor = actor(&e);
    let ttls = client.subsidize(&sponsor, &usd, &eur, &MARKET_LISTING_FEE);

    assert_eq!(ttls.len(), 1);
    let days = MARKET_LISTING_FEE / ORACLE_DAILY_FEE;
    assert_eq!(
        tracked_until(&e, &axis, &usd),
        usd_access + days as u64 * 86_400
    );
    //the delisted asset gained nothing, and the record now says so
    assert_eq!(tracked_until(&e, &axis, &eur), eur_access);
    assert!(!client.market(&usd, &eur).unwrap().side(&eur).listed);
}

#[test]
fn test_subsidize_rejects_a_fully_delisted_pair() {
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    create_sell_order(&client, &trader, 1000, &usd, &eur);

    delist(&e, &usd);
    delist(&e, &eur);
    let sponsor = actor(&e);
    let before = xrf_balance(&e, &sponsor);

    assert_eq!(
        code(client.try_subsidize(&sponsor, &usd, &eur, &MARKET_LISTING_FEE)),
        Some(721)
    );
    assert_eq!(xrf_balance(&e, &sponsor), before);
}
