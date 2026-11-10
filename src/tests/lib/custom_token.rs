//! Trading a custom SEP-41 token that is not a Stellar Asset Contract: `test-token`, a generic
//! token built from this repository (`wasm/test_token.wasm`, rebuild it with the command in
//! `test-token/src/lib.rs`), set up with 8 decimals. It answers no `authorized`, so the contract
//! treats every account as able to receive it. It keeps its own allowance expiration, and its
//! admin can pause transfers and blacklist accounts. Paused transfers and blacklisted accounts
//! show up only when a transfer runs: a maker whose tokens cannot be collected is skipped, a
//! payment in the token that cannot be made fails the call with the token's error.
use super::setup::{
    actor, approval, approve, assert_no_custody, assert_trades_match_balances, balance, balances,
    code, fake_asset, fund, list_asset, no_approvals, no_orders, nonce, open_market, order_update,
    register_axis, removal, setup_test, store_order, trade, try_trade, try_update_one, update_one,
    ORACLE_DECIMALS,
};
use crate::events::{OrderSkippedEvent, TradeEvent};
use crate::order::{OrderKind, TradeDirection};
use crate::trade::{Approval, TradeStep};
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, Env, Event, String, Vec};

mod test_token {
    soroban_sdk::contractimport!(file = "src/tests/lib/wasm/test_token.wasm");
}

/// Test token decimals
pub const TKN_DECIMALS: u32 = 8;
/// 1 TKN
pub const TKN: i128 = 10i128.pow(TKN_DECIMALS);
/// 1 USDC, a Stellar asset with 7 decimals
pub const USDC: i128 = 10i128.pow(7);
/// Oracle quote of 1 TKN: 100,000 USD
pub const TKN_QUOTE: i128 = 100_000 * 10i128.pow(ORACLE_DECIMALS);
/// Oracle quote of 1 USDC: 1 USD
pub const USDC_QUOTE: i128 = 10i128.pow(ORACLE_DECIMALS);
/// Order price of TKN in USDC: 100,000 USDC (10^12 units) per TKN (10^8 units)
pub const TKN_PRICE: i128 = 10_000 * PRECISION;
/// Order price of USDC in TKN, the exact inverse of `TKN_PRICE`
pub const USDC_PRICE: i128 = PRECISION / 10_000;
/// Fewest TKN units worth the 1 USD minimum trade size (0.00001 TKN)
const MIN_TKN_ORDER: i128 = TKN / 100_000;

/// `TokenError::Paused` of the test token
const PAUSED: u32 = test_token::TokenError::Paused as u32;
/// `TokenError::Blacklisted` of the test token
const BLACKLISTED: u32 = test_token::TokenError::Blacklisted as u32;

/// A test token deployment quoted by the oracle at `TKN_QUOTE`
pub struct CustomToken<'a> {
    pub address: Address,
    client: test_token::Client<'a>,
}

impl<'a> CustomToken<'a> {
    pub fn deploy(e: &'a Env) -> Self {
        let admin = Address::generate(e);
        let address = e.register(
            test_token::WASM,
            (
                admin,
                TKN_DECIMALS,
                String::from_str(e, "Test token"),
                String::from_str(e, "TKN"),
            ),
        );
        let client = test_token::Client::new(e, &address);
        list_asset(e, &address, TKN_QUOTE);
        CustomToken { address, client }
    }

    pub fn mint(&self, account: &Address, amount: i128) {
        self.client.mint(account, &amount);
    }

    /// Mint `amount` to `account` and let `axis` spend anything the account holds
    pub fn fund(&self, axis: &Address, account: &Address, amount: i128) {
        self.mint(account, amount);
        approve(&self.client.env, &self.address, axis, account, i128::MAX);
    }

    pub fn allowance(&self, owner: &Address, spender: &Address) -> i128 {
        self.client.allowance(owner, spender)
    }

    pub fn pause(&self) {
        self.client.set_paused(&true);
    }

    pub fn unpause(&self) {
        self.client.set_paused(&false);
    }

    pub fn blacklist(&self, account: &Address) {
        self.client.set_blacklisted(account, &true);
    }
}

/// AXIS with an open TKN/USDC market, both assets quoted and their prices cached
struct Book<'a> {
    client: AxisClient<'a>,
    axis: Address,
    tkn: CustomToken<'a>,
    usdc: Address,
}

fn book<'a>(e: &'a Env, issuer: &Address) -> Book<'a> {
    let tkn = CustomToken::deploy(e);
    let usdc = fake_asset(e, issuer);
    list_asset(e, &usdc, USDC_QUOTE);
    let axis = register_axis(e);
    let client = AxisClient::new(e, &axis);
    open_market(&client, &tkn.address, &usdc);
    Book {
        client,
        axis,
        tkn,
        usdc,
    }
}

/// `trade` with a fresh nonce and an in-call approval
#[allow(clippy::too_many_arguments)]
fn try_trade_approving(
    b: &Book,
    direction: TradeDirection,
    kind: OrderKind,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
    orders: &Vec<u128>,
    approve: Approval,
) -> Result<(i128, i128, Option<u128>), Option<u32>> {
    match b.client.try_trade(
        &direction,
        &kind,
        trader,
        &amount,
        selling,
        buying,
        &price,
        orders,
        &nonce(),
        &0,
        &Some(approve),
    ) {
        Ok(Ok(res)) => Ok(res),
        res => Err(code(res)),
    }
}

// ---- markets and order creation --------------------------------------------------------------

#[test]
fn test_market_reads_the_token_decimals() {
    let (e, _, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let market = b.client.market(&b.tkn.address, &b.usdc).unwrap();
    let side = market.side(&b.tkn.address);
    assert!(side.listed);
    assert_eq!(side.decimals, TKN_DECIMALS);
    assert_eq!(market.side(&b.usdc).decimals, 7);
}

#[test]
fn test_sell_order_backed_by_an_in_call_approval() {
    // The minimum trade size of 1 USD is 1000 TKN units at 100,000 USD per TKN: the
    // valuation reads the token's own 8 decimals. Creating the order moves nothing, the approval
    // granted in the call goes through the token's own `approve`
    let (e, _, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.mint(&maker, TKN);
    let sell = |amount: i128| {
        try_trade_approving(
            &b,
            TradeDirection::Sell,
            OrderKind::Limit,
            &maker,
            amount,
            &b.tkn.address,
            &b.usdc,
            TKN_PRICE,
            &no_orders(&e),
            approval(&e, &b.tkn.address, amount),
        )
    };
    assert_eq!(sell(MIN_TKN_ORDER - 1), Err(Some(720)));
    let (sold, bought, id) = sell(MIN_TKN_ORDER).unwrap();
    assert_eq!((sold, bought), (0, 0));
    let order = b.client.order(&id.unwrap()).unwrap();
    assert_eq!(order.selling, b.tkn.address);
    assert_eq!(order.buying, b.usdc);
    assert_eq!(order.amount, MIN_TKN_ORDER);
    assert_eq!(order.price, TKN_PRICE);
    assert_eq!(b.tkn.allowance(&maker, &b.axis), MIN_TKN_ORDER);
    assert_eq!(balance(&e, &b.tkn.address, &maker), TKN);
    assert_no_custody(&e, &b.axis, &[&b.tkn.address, &b.usdc]);
}

#[test]
fn test_sell_order_rejected_without_backing() {
    let (e, _, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.mint(&maker, TKN);
    let sell = |amount: i128| {
        try_trade(
            &b.client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &maker,
            amount,
            &b.tkn.address,
            &b.usdc,
            TKN_PRICE,
            &no_orders(&e),
        )
    };
    // no allowance yet, then more than the balance
    assert_eq!(sell(TKN), Some(703));
    approve(&e, &b.tkn.address, &b.axis, &maker, i128::MAX);
    assert_eq!(sell(TKN + 1), Some(702));
    assert_eq!(sell(TKN), None);
}

#[test]
fn test_minimum_value_read_on_the_bought_token() {
    // the partner asset has no quote, so the order is valued by the tokens it buys, in 8 decimals
    let (e, trader, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let unlisted = fake_asset(&e, &issuer);
    open_market(&b.client, &unlisted, &b.tkn.address);
    fund(&e, &unlisted, &b.axis, &trader, TKN);
    let sell = |amount: i128| {
        try_trade(
            &b.client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &trader,
            amount,
            &unlisted,
            &b.tkn.address,
            PRECISION,
            &no_orders(&e),
        )
    };
    assert_eq!(sell(MIN_TKN_ORDER - 1), Some(720));
    assert_eq!(sell(MIN_TKN_ORDER), None);
}

#[test]
fn test_update_backed_by_an_in_call_approval() {
    let (e, _, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.mint(&maker, TKN);
    let (_, _, id) = try_trade_approving(
        &b,
        TradeDirection::Sell,
        OrderKind::Limit,
        &maker,
        TKN / 10,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &no_orders(&e),
        approval(&e, &b.tkn.address, TKN / 10),
    )
    .unwrap();
    let id = id.unwrap();
    // the allowance covers 0.1 TKN only
    assert_eq!(
        try_update_one(&b.client, &maker, order_update(id, TKN / 5, TKN_PRICE)),
        Some(703)
    );
    let updates = Vec::from_array(&e, [order_update(id, TKN / 5, TKN_PRICE)]);
    let approvals = Vec::from_array(&e, [approval(&e, &b.tkn.address, TKN / 5)]);
    b.client.update(&maker, &updates, &approvals);
    assert_eq!(b.client.order(&id).unwrap().amount, TKN / 5);
    assert_eq!(b.tkn.allowance(&maker, &b.axis), TKN / 5);
}

// ---- settlement -------------------------------------------------------------------------------

#[test]
fn test_buy_the_token_from_a_maker() {
    // The maker's tokens pass through the contract (`transfer_from` maker -> contract, then
    // `transfer` contract -> taker), the taker pays the maker in USDC directly
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.fund(&b.axis, &maker, TKN);
    fund(&e, &b.usdc, &b.axis, &taker, 100_000 * USDC);
    let id = store_order(
        &b.client,
        &maker,
        TKN / 2,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );
    let accounts = [&taker, &maker, &b.axis];
    let assets = [&b.tkn.address, &b.usdc];
    let before = balances(&e, &accounts, &assets);

    let res = trade(
        &b.client,
        TradeDirection::Buy,
        OrderKind::Fill,
        &taker,
        TKN / 5,
        &b.usdc,
        &b.tkn.address,
        TKN_PRICE,
        &Vec::from_array(&e, [id]),
    );
    let events = e.events().all().filter_by_contract(&b.axis);
    assert_eq!(res, (20_000 * USDC, TKN / 5, None));
    let fill = TradeEvent {
        selling: b.usdc.clone(),
        buying: b.tkn.address.clone(),
        order: id,
        taker: taker.clone(),
        maker: maker.clone(),
        sold: 20_000 * USDC,
        bought: TKN / 5,
        left: 3 * TKN / 10,
    };
    assert_eq!(events, [fill.to_xdr(&e, &b.axis)]);
    assert_trades_match_balances(&e, &[fill], &accounts, &assets, &before);
    assert_eq!(b.client.order(&id).unwrap().amount, 3 * TKN / 10);
    assert_no_custody(&e, &b.axis, &assets);
}

#[test]
fn test_sell_the_token_to_a_maker_with_an_in_call_approval() {
    // The taker pays the maker in the token with `transfer_from` against the allowance granted in
    // the same call, which the fill spends exactly
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    fund(&e, &b.usdc, &b.axis, &maker, 50_000 * USDC);
    b.tkn.mint(&taker, TKN);
    let id = store_order(
        &b.client,
        &maker,
        50_000 * USDC,
        &b.usdc,
        &b.tkn.address,
        USDC_PRICE,
    );
    let accounts = [&taker, &maker, &b.axis];
    let assets = [&b.tkn.address, &b.usdc];
    let before = balances(&e, &accounts, &assets);

    let res = try_trade_approving(
        &b,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        TKN / 4,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &Vec::from_array(&e, [id]),
        approval(&e, &b.tkn.address, TKN / 4),
    );
    let events = e.events().all().filter_by_contract(&b.axis);
    assert_eq!(res, Ok((TKN / 4, 25_000 * USDC, None)));
    let fill = TradeEvent {
        selling: b.tkn.address.clone(),
        buying: b.usdc.clone(),
        order: id,
        taker: taker.clone(),
        maker: maker.clone(),
        sold: TKN / 4,
        bought: 25_000 * USDC,
        left: 25_000 * USDC,
    };
    assert_eq!(events, [fill.to_xdr(&e, &b.axis)]);
    assert_trades_match_balances(&e, &[fill], &accounts, &assets, &before);
    assert_eq!(b.tkn.allowance(&taker, &b.axis), 0);
    assert_no_custody(&e, &b.axis, &assets);
}

#[test]
fn test_remainder_buying_the_token_is_stored_and_filled() {
    // A Buy Limit for 0.5 TKN fills 0.2 from the book; the remainder is stored in
    // sell-equivalent form (30,000 USDC for TKN at the inverse price), then a seller fills it
    // and its owner is paid in TKN
    let (e, trader, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    let seller = actor(&e);
    b.tkn.fund(&b.axis, &maker, TKN / 5);
    b.tkn.fund(&b.axis, &seller, TKN);
    fund(&e, &b.usdc, &b.axis, &trader, 50_000 * USDC);
    let id = store_order(
        &b.client,
        &maker,
        TKN / 5,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );

    let (sold, bought, created) = trade(
        &b.client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        TKN / 2,
        &b.usdc,
        &b.tkn.address,
        TKN_PRICE,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (20_000 * USDC, TKN / 5));
    let created = created.unwrap();
    let order = b.client.order(&created).unwrap();
    assert_eq!(order.selling, b.usdc);
    assert_eq!(order.buying, b.tkn.address);
    assert_eq!(order.amount, 30_000 * USDC);
    assert_eq!(order.price, USDC_PRICE);

    let res = trade(
        &b.client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &seller,
        TKN,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &Vec::from_array(&e, [created]),
    );
    assert_eq!(res, (3 * TKN / 10, 30_000 * USDC, None));
    assert!(b.client.order(&created).is_none());
    assert_eq!(balance(&e, &b.tkn.address, &trader), TKN / 2);
    assert_eq!(balance(&e, &b.usdc, &trader), 0);
    assert_eq!(balance(&e, &b.tkn.address, &seller), 7 * TKN / 10);
    assert_no_custody(&e, &b.axis, &[&b.tkn.address, &b.usdc]);
}

#[test]
fn test_swap_routes_through_the_token() {
    // USDC -> TKN -> EURC: the contract holds the tokens bought on the first hop only within
    // the call, paying the second maker with a plain `transfer`
    let (e, trader, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let eurc = fake_asset(&e, &issuer);
    list_asset(&e, &eurc, USDC_QUOTE);
    let tkn_maker = actor(&e);
    let eurc_maker = actor(&e);
    b.tkn.fund(&b.axis, &tkn_maker, TKN);
    fund(&e, &eurc, &b.axis, &eurc_maker, 50_000 * USDC);
    fund(&e, &b.usdc, &b.axis, &trader, 10_000 * USDC);
    let tkn_order = store_order(
        &b.client,
        &tkn_maker,
        TKN,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );
    // EURC at par with USDC
    let eurc_order = store_order(
        &b.client,
        &eurc_maker,
        50_000 * USDC,
        &eurc,
        &b.tkn.address,
        USDC_PRICE,
    );
    let path = Vec::from_array(
        &e,
        [
            TradeStep {
                asset: b.tkn.address.clone(),
                orders: Vec::from_array(&e, [tkn_order]),
            },
            TradeStep {
                asset: eurc.clone(),
                orders: Vec::from_array(&e, [eurc_order]),
            },
        ],
    );
    let res = b.client.swap(
        &TradeDirection::Sell,
        &trader,
        &b.usdc,
        &(10_000 * USDC),
        &(10_000 * USDC),
        &path,
        &None,
    );
    assert_eq!(res, (10_000 * USDC, 10_000 * USDC));
    assert_eq!(balance(&e, &eurc, &trader), 10_000 * USDC);
    assert_eq!(balance(&e, &b.usdc, &tkn_maker), 10_000 * USDC);
    assert_eq!(balance(&e, &b.tkn.address, &eurc_maker), TKN / 10);
    assert_eq!(b.client.order(&tkn_order).unwrap().amount, 9 * TKN / 10);
    assert_eq!(b.client.order(&eurc_order).unwrap().amount, 40_000 * USDC);
    assert_no_custody(&e, &b.axis, &[&b.tkn.address, &b.usdc, &eurc]);
}

#[test]
fn test_crossfill_pays_out_the_token_from_the_contract() {
    // The taker order pays 100,000 USDC per TKN, the maker asks 80,000: the contract collects
    // the maker's 0.125 TKN, pays the owner the 0.1 TKN it asks for and the caller the
    // 0.025 TKN spread, all with plain `transfer`s out of its own balance
    let (e, _, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let owner = actor(&e);
    let maker = actor(&e);
    let arbitrageur = actor(&e);
    fund(&e, &b.usdc, &b.axis, &owner, 10_000 * USDC);
    b.tkn.fund(&b.axis, &maker, TKN / 5);
    let taker_order = store_order(
        &b.client,
        &owner,
        10_000 * USDC,
        &b.usdc,
        &b.tkn.address,
        USDC_PRICE,
    );
    let maker_order = store_order(
        &b.client,
        &maker,
        TKN / 5,
        &b.tkn.address,
        &b.usdc,
        8_000 * PRECISION,
    );
    let res = b.client.crossfill(
        &arbitrageur,
        &taker_order,
        &Vec::from_array(&e, [maker_order]),
    );
    assert_eq!(res, (10_000 * USDC, TKN / 8, TKN / 40));
    assert_eq!(balance(&e, &b.tkn.address, &owner), TKN / 10);
    assert_eq!(balance(&e, &b.tkn.address, &arbitrageur), TKN / 40);
    assert_eq!(balance(&e, &b.usdc, &maker), 10_000 * USDC);
    assert!(b.client.order(&taker_order).is_none());
    assert_eq!(b.client.order(&maker_order).unwrap().amount, 3 * TKN / 40);
    assert_no_custody(&e, &b.axis, &[&b.tkn.address, &b.usdc]);
}

// ---- token-side restrictions ------------------------------------------------------------------

#[test]
fn test_expired_allowance_skips_the_maker_until_renewed() {
    // The token expires the allowance itself: past its ledger the maker's order has no backing and
    // is skipped untouched; an approval-only `update` revives it
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.mint(&maker, TKN);
    fund(&e, &b.usdc, &b.axis, &taker, 100_000 * USDC);
    let short_lived = Approval {
        asset: b.tkn.address.clone(),
        amount: TKN,
        live_until: e.ledger().sequence() + 100,
    };
    let (_, _, id) = try_trade_approving(
        &b,
        TradeDirection::Sell,
        OrderKind::Limit,
        &maker,
        TKN / 2,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &no_orders(&e),
        short_lived,
    )
    .unwrap();
    let id = id.unwrap();
    e.ledger().set_sequence_number(e.ledger().sequence() + 101);
    assert_eq!(b.tkn.allowance(&maker, &b.axis), 0);

    let buy = || {
        trade(
            &b.client,
            TradeDirection::Buy,
            OrderKind::Fill,
            &taker,
            TKN / 5,
            &b.usdc,
            &b.tkn.address,
            TKN_PRICE,
            &Vec::from_array(&e, [id]),
        )
    };
    assert_eq!(buy(), (0, 0, None));
    let events = e.events().all().filter_by_contract(&b.axis);
    assert_eq!(
        events,
        [OrderSkippedEvent { order: id }.to_xdr(&e, &b.axis)]
    );
    assert_eq!(b.client.order(&id).unwrap().amount, TKN / 2);

    let approvals = Vec::from_array(&e, [approval(&e, &b.tkn.address, TKN)]);
    b.client.update(&maker, &Vec::new(&e), &approvals);
    assert_eq!(buy(), (20_000 * USDC, TKN / 5, None));
}

#[test]
fn test_paused_token_skips_the_makers_selling_it() {
    // Collecting the makers' tokens fails while the token is paused, so their orders are skipped
    // untouched: a Fill buys nothing, a FillOrKill fails, a Limit stores the whole amount
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.fund(&b.axis, &maker, TKN);
    fund(&e, &b.usdc, &b.axis, &taker, 100_000 * USDC);
    let id = store_order(
        &b.client,
        &maker,
        TKN / 2,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );
    let orders = Vec::from_array(&e, [id]);
    b.tkn.pause();

    let buy = |kind: OrderKind| {
        try_trade(
            &b.client,
            TradeDirection::Buy,
            kind,
            &taker,
            TKN / 5,
            &b.usdc,
            &b.tkn.address,
            TKN_PRICE,
            &orders,
        )
    };
    let res = trade(
        &b.client,
        TradeDirection::Buy,
        OrderKind::Fill,
        &taker,
        TKN / 5,
        &b.usdc,
        &b.tkn.address,
        TKN_PRICE,
        &orders,
    );
    let events = e.events().all().filter_by_contract(&b.axis);
    assert_eq!(res, (0, 0, None));
    assert_eq!(
        events,
        [OrderSkippedEvent { order: id }.to_xdr(&e, &b.axis)]
    );
    assert_eq!(buy(OrderKind::FillOrKill), Some(709));
    let (sold, bought, created) = trade(
        &b.client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &taker,
        TKN / 5,
        &b.usdc,
        &b.tkn.address,
        TKN_PRICE,
        &orders,
    );
    assert_eq!((sold, bought), (0, 0));
    assert_eq!(
        b.client.order(&created.unwrap()).unwrap().amount,
        20_000 * USDC
    );
    assert_eq!(b.client.order(&id).unwrap().amount, TKN / 2);
    assert_eq!(balance(&e, &b.tkn.address, &maker), TKN);
    assert_eq!(balance(&e, &b.usdc, &taker), 100_000 * USDC);

    b.tkn.unpause();
    assert_eq!(buy(OrderKind::Fill), None);
    assert_eq!(b.client.order(&id).unwrap().amount, 3 * TKN / 10);
}

#[test]
fn test_paused_token_fails_trades_paying_with_it() {
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    fund(&e, &b.usdc, &b.axis, &maker, 50_000 * USDC);
    b.tkn.fund(&b.axis, &taker, TKN);
    let id = store_order(
        &b.client,
        &maker,
        50_000 * USDC,
        &b.usdc,
        &b.tkn.address,
        USDC_PRICE,
    );
    let sell = || {
        try_trade(
            &b.client,
            TradeDirection::Sell,
            OrderKind::Fill,
            &taker,
            TKN / 4,
            &b.tkn.address,
            &b.usdc,
            TKN_PRICE,
            &Vec::from_array(&e, [id]),
        )
    };
    b.tkn.pause();
    assert_eq!(sell(), Some(PAUSED));
    assert_eq!(b.client.order(&id).unwrap().amount, 50_000 * USDC);
    assert_eq!(balance(&e, &b.usdc, &maker), 50_000 * USDC);

    b.tkn.unpause();
    assert_eq!(sell(), None);
}

#[test]
fn test_paused_token_keeps_order_management() {
    // Approvals, balance and allowance reads work while the token is paused, so orders are created,
    // changed and removed as usual
    let (e, _, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.mint(&maker, TKN);
    b.tkn.pause();
    let (_, _, id) = try_trade_approving(
        &b,
        TradeDirection::Sell,
        OrderKind::Limit,
        &maker,
        TKN / 2,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &no_orders(&e),
        approval(&e, &b.tkn.address, TKN),
    )
    .unwrap();
    let id = id.unwrap();
    update_one(&b.client, &maker, order_update(id, TKN, TKN_PRICE));
    assert_eq!(b.client.order(&id).unwrap().amount, TKN);
    let removals = Vec::from_array(&e, [removal(id)]);
    b.client.update(&maker, &removals, &no_approvals(&e));
    assert!(b.client.order(&id).is_none());
}

#[test]
fn test_blacklisted_maker_selling_the_token_is_skipped_at_settlement() {
    // The blacklist is invisible to the admission checks (the balance and the allowance read
    // fine), so the blacklisted maker's fill is planned and only fails when its tokens are
    // collected. The planned fill has used up the amount by then: with just enough asked for one
    // order the trade buys nothing, the order behind it is never matched
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let bad = actor(&e);
    let good = actor(&e);
    b.tkn.fund(&b.axis, &bad, TKN);
    b.tkn.fund(&b.axis, &good, TKN);
    fund(&e, &b.usdc, &b.axis, &taker, 100_000 * USDC);
    let bad_order = store_order(&b.client, &bad, TKN / 5, &b.tkn.address, &b.usdc, TKN_PRICE);
    let good_order = store_order(
        &b.client,
        &good,
        TKN / 5,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );
    b.tkn.blacklist(&bad);
    let orders = Vec::from_array(&e, [bad_order, good_order]);
    let buy = |amount: i128| {
        trade(
            &b.client,
            TradeDirection::Buy,
            OrderKind::Fill,
            &taker,
            amount,
            &b.usdc,
            &b.tkn.address,
            TKN_PRICE,
            &orders,
        )
    };

    assert_eq!(buy(TKN / 5), (0, 0, None));
    let events = e.events().all().filter_by_contract(&b.axis);
    assert_eq!(
        events,
        [OrderSkippedEvent { order: bad_order }.to_xdr(&e, &b.axis)]
    );
    assert_eq!(b.client.order(&good_order).unwrap().amount, TKN / 5);

    // asking for both orders' worth reaches the second one
    let res = buy(2 * TKN / 5);
    let events = e.events().all().filter_by_contract(&b.axis);
    assert_eq!(res, (20_000 * USDC, TKN / 5, None));
    assert!(events
        .events()
        .contains(&OrderSkippedEvent { order: bad_order }.to_xdr(&e, &b.axis)));
    assert_eq!(b.client.order(&bad_order).unwrap().amount, TKN / 5);
    assert!(b.client.order(&good_order).is_none());
    assert_eq!(balance(&e, &b.tkn.address, &bad), TKN);
    assert_no_custody(&e, &b.axis, &[&b.tkn.address, &b.usdc]);
}

#[test]
fn test_blacklisted_maker_buying_the_token_fails_the_trades_listing_it() {
    // Paying a blacklisted maker in the token fails, and the contract cannot tell it from a taker
    // who cannot pay (`can_receive` sees no `authorized`): the call fails with the token's error.
    // Trades that leave the order out are unaffected
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let bad = actor(&e);
    let good = actor(&e);
    fund(&e, &b.usdc, &b.axis, &bad, 50_000 * USDC);
    fund(&e, &b.usdc, &b.axis, &good, 50_000 * USDC);
    b.tkn.fund(&b.axis, &taker, TKN);
    let bad_order = store_order(
        &b.client,
        &bad,
        50_000 * USDC,
        &b.usdc,
        &b.tkn.address,
        USDC_PRICE,
    );
    let good_order = store_order(
        &b.client,
        &good,
        50_000 * USDC,
        &b.usdc,
        &b.tkn.address,
        USDC_PRICE,
    );
    b.tkn.blacklist(&bad);
    let sell = |orders: &[u128]| {
        try_trade(
            &b.client,
            TradeDirection::Sell,
            OrderKind::Fill,
            &taker,
            TKN / 4,
            &b.tkn.address,
            &b.usdc,
            TKN_PRICE,
            &Vec::from_slice(&e, orders),
        )
    };
    assert_eq!(sell(&[bad_order, good_order]), Some(BLACKLISTED));
    assert_eq!(b.client.order(&bad_order).unwrap().amount, 50_000 * USDC);
    assert_eq!(balance(&e, &b.usdc, &bad), 50_000 * USDC);
    assert_eq!(sell(&[good_order]), None);
    assert_eq!(b.client.order(&good_order).unwrap().amount, 25_000 * USDC);
}

#[test]
fn test_blacklisted_taker_cannot_receive_the_token() {
    // The makers settle, then forwarding their tokens to the taker fails: `CannotReceive`
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let maker = actor(&e);
    b.tkn.fund(&b.axis, &maker, TKN);
    fund(&e, &b.usdc, &b.axis, &taker, 100_000 * USDC);
    let id = store_order(
        &b.client,
        &maker,
        TKN / 2,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );
    b.tkn.blacklist(&taker);
    assert_eq!(
        try_trade(
            &b.client,
            TradeDirection::Buy,
            OrderKind::Fill,
            &taker,
            TKN / 5,
            &b.usdc,
            &b.tkn.address,
            TKN_PRICE,
            &Vec::from_array(&e, [id]),
        ),
        Some(708)
    );
    assert_eq!(b.client.order(&id).unwrap().amount, TKN / 2);
    assert_eq!(balance(&e, &b.tkn.address, &maker), TKN);
    assert_no_custody(&e, &b.axis, &[&b.tkn.address, &b.usdc]);
}

#[test]
fn test_blacklisted_contract_stops_trading_the_token() {
    // With the DEX contract itself blacklisted, approvals to it fail and no token moves through
    // it: makers selling the token are skipped, payments in the token fail the call
    let (e, taker, issuer, _, _) = setup_test();
    let b = book(&e, &issuer);
    let seller = actor(&e);
    let buyer = actor(&e);
    b.tkn.fund(&b.axis, &seller, TKN);
    fund(&e, &b.usdc, &b.axis, &buyer, 50_000 * USDC);
    fund(&e, &b.usdc, &b.axis, &taker, 100_000 * USDC);
    b.tkn.fund(&b.axis, &taker, TKN);
    let ask = store_order(
        &b.client,
        &seller,
        TKN / 5,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
    );
    let bid = store_order(
        &b.client,
        &buyer,
        50_000 * USDC,
        &b.usdc,
        &b.tkn.address,
        USDC_PRICE,
    );
    b.tkn.blacklist(&b.axis);

    let approving = try_trade_approving(
        &b,
        TradeDirection::Sell,
        OrderKind::Limit,
        &taker,
        TKN / 10,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &no_orders(&e),
        approval(&e, &b.tkn.address, TKN),
    );
    assert_eq!(approving, Err(Some(BLACKLISTED)));
    let buy = trade(
        &b.client,
        TradeDirection::Buy,
        OrderKind::Fill,
        &taker,
        TKN / 5,
        &b.usdc,
        &b.tkn.address,
        TKN_PRICE,
        &Vec::from_array(&e, [ask]),
    );
    assert_eq!(buy, (0, 0, None));
    let sell = try_trade(
        &b.client,
        TradeDirection::Sell,
        OrderKind::Fill,
        &taker,
        TKN / 4,
        &b.tkn.address,
        &b.usdc,
        TKN_PRICE,
        &Vec::from_array(&e, [bid]),
    );
    assert_eq!(sell, Some(BLACKLISTED));
}
