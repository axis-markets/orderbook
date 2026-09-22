use super::mock_oracle::{MockOracle, MockOracleClient};
use crate::market::Config;
use crate::order::{OrderKind, TradeDirection};
use crate::reflector_beam::Asset;
use crate::tests::lib::utils::shorten;
use crate::trade::{Approval, OrderUpdate};
use crate::{Axis, AxisClient};
use core::sync::atomic::{AtomicU64, Ordering};
use soroban_sdk::testutils::{Address as _, IssuerFlags, Ledger as _};
use soroban_sdk::{log, token, Address, Env, InvokeError, Vec};

/// Oracle price decimals (Reflector mainnet default)
pub const ORACLE_DECIMALS: u32 = 14;
/// Stellar asset decimals
pub const TOKEN_DECIMALS: u32 = 7;
/// Oracle price at which 1 token base unit (stroop) is worth exactly 1 USD
pub const UNIT_PRICE: i128 = 10i128.pow(ORACLE_DECIMALS + TOKEN_DECIMALS);
/// Mock oracle daily fee per asset (1 XRF)
pub const ORACLE_DAILY_FEE: i128 = 1_0000000;
/// XRF burned to open a market, derived by the contract from the oracle's daily fee (90 XRF)
pub const MARKET_LISTING_FEE: i128 = ORACLE_DAILY_FEE * crate::market::LISTING_FEE_DAYS;
/// Initial ledger timestamp
pub const START_TIMESTAMP: u64 = 1_700_000_000;
/// Minimum trade size configured in tests (1 USD, 7 decimals)
pub const MIN_TRADE_SIZE: i128 = 10i128.pow(7);
/// Mainnet minimum persistent entry TTL (ledgers)
pub const MIN_PERSISTENT_TTL: u32 = 2_073_600;
/// Mainnet maximum entry TTL (ledgers)
pub const MAX_ENTRY_TTL: u32 = 3_110_400;
/// Fixed test oracle contract id
const ORACLE_ID: &str = "CBLLEW7HD2RWATVSMLAGWM4G3WCHSHDJ25ALP4DI6LULV5TU35N2CIZA";
/// Fixed test safety admin address
const SAFETY_ADMIN_ID: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

static NONCE: AtomicU64 = AtomicU64::new(1);

/// Fresh client nonce for a new order
pub fn nonce() -> u64 {
    NONCE.fetch_add(1, Ordering::Relaxed)
}

/// Create a fake Stellar asset for testing
pub fn fake_asset(env: &Env, issuer: &Address) -> Address {
    env.register_stellar_asset_contract_v2(issuer.clone())
        .address()
}

/// Create a fake Stellar asset whose issuer may revoke balance authorization
pub fn fake_asset_revocable(env: &Env, issuer: &Address) -> Address {
    let sac = env.register_stellar_asset_contract_v2(issuer.clone());
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    sac.address()
}

/// Oracle contract address
pub fn oracle_address(e: &Env) -> Address {
    Address::from_str(e, ORACLE_ID)
}

/// Safety admin address wired into the test contract configuration
pub fn safety_admin(e: &Env) -> Address {
    Address::from_str(e, SAFETY_ADMIN_ID)
}

/// Mock oracle client
pub fn oracle_client(e: &Env) -> MockOracleClient<'_> {
    MockOracleClient::new(e, &oracle_address(e))
}

/// Register the XRF fee token and the mock oracle. Returns the XRF token address
pub fn setup_oracle(e: &Env, issuer: &Address) -> Address {
    let xrf = fake_asset(e, issuer);
    e.register_at(
        &oracle_address(e),
        MockOracle,
        (xrf.clone(), ORACLE_DECIMALS, ORACLE_DAILY_FEE),
    );
    xrf
}

/// Register a second mock oracle at a fresh address, charging the same XRF fee token but
/// quoting with `decimals`. Used to exercise an oracle handover
pub fn register_oracle(e: &Env, decimals: u32) -> Address {
    e.register(MockOracle, (xrf(e), decimals, ORACLE_DAILY_FEE))
}

/// XRF fee token address
pub fn xrf(e: &Env) -> Address {
    oracle_client(e).fee_token()
}

/// Mint enough XRF to `account` to open several markets
pub fn fund_xrf(e: &Env, account: &Address) {
    token::StellarAssetClient::new(e, &xrf(e))
        .mock_all_auths()
        .mint(account, &(MARKET_LISTING_FEE * 10));
}

/// Generate an address holding XRF
pub fn actor(e: &Env) -> Address {
    let account = Address::generate(e);
    fund_xrf(e, &account);
    account
}

/// Last ledger an allowance may live until
pub fn max_live_until(e: &Env) -> u32 {
    let info = e.ledger().get();
    info.sequence_number + info.max_entry_ttl - 1
}

/// Grant `axis` an allowance of `amount` on `asset` from `account`, valid as long as possible
pub fn approve(e: &Env, asset: &Address, axis: &Address, account: &Address, amount: i128) {
    token::Client::new(e, asset).approve(account, axis, &amount, &max_live_until(e));
}

/// Mint `amount` of `asset` to `account` and let `axis` spend anything the account holds
pub fn fund(e: &Env, asset: &Address, axis: &Address, account: &Address, amount: i128) {
    token::StellarAssetClient::new(e, asset).mint(account, &amount);
    approve(e, asset, axis, account, i128::MAX);
}

/// Token balance
pub fn balance(e: &Env, asset: &Address, account: &Address) -> i128 {
    token::Client::new(e, asset).balance(account)
}

/// The contract holds none of `assets`
pub fn assert_no_custody(e: &Env, axis: &Address, assets: &[&Address]) {
    for asset in assets {
        assert_eq!(
            balance(e, asset, axis),
            0,
            "contract holds {}",
            shorten(asset)
        );
    }
}

/// Contract error code of a failed call, `None` when the call succeeded
pub fn code<T>(res: Result<T, Result<soroban_sdk::Error, InvokeError>>) -> Option<u32> {
    match res {
        Ok(_) => None,
        Err(Ok(err)) => Some(err.get_code()),
        Err(Err(err)) => panic!("unexpected invoke error {:?}", err),
    }
}

/// Empty order list
pub fn no_orders(e: &Env) -> Vec<u128> {
    Vec::new(e)
}

/// An allowance of `amount` on `asset` granted in the call, valid as long as possible
pub fn approval(e: &Env, asset: &Address, amount: i128) -> Approval {
    Approval {
        asset: asset.clone(),
        amount,
        live_until: max_live_until(e),
    }
}

/// No allowance changes for `update`
pub fn no_approvals(e: &Env) -> Vec<Approval> {
    Vec::new(e)
}

/// New amount and price for an order, without an expiration
pub fn order_update(id: u128, amount: i128, price: i128) -> OrderUpdate {
    OrderUpdate {
        id,
        amount,
        price,
        expires: 0,
    }
}

/// An update removing the order (zero amount)
pub fn removal(id: u128) -> OrderUpdate {
    order_update(id, 0, 0)
}

/// `update` of a single order without approvals, returning the ids updated
pub fn update_one(client: &AxisClient, trader: &Address, update: OrderUpdate) -> Vec<u128> {
    let updates = Vec::from_array(&client.env, [update]);
    client.update(trader, &updates, &no_approvals(&client.env))
}

/// `try_update` of a single order without approvals, returning the error code on failure
pub fn try_update_one(client: &AxisClient, trader: &Address, update: OrderUpdate) -> Option<u32> {
    let updates = Vec::from_array(&client.env, [update]);
    code(client.try_update(trader, &updates, &no_approvals(&client.env)))
}

/// Remove orders through `update`, returning the ids removed
pub fn remove_orders(client: &AxisClient, trader: &Address, ids: &[u128]) -> Vec<u128> {
    let mut updates = Vec::new(&client.env);
    for id in ids {
        updates.push_back(removal(*id));
    }
    client.update(trader, &updates, &no_approvals(&client.env))
}

/// `try_update` removing orders, returning the error code on failure
pub fn try_remove_orders(client: &AxisClient, trader: &Address, ids: &[u128]) -> Option<u32> {
    let mut updates = Vec::new(&client.env);
    for id in ids {
        updates.push_back(removal(*id));
    }
    code(client.try_update(trader, &updates, &no_approvals(&client.env)))
}

/// `trade` with a fresh nonce and no approval
#[allow(clippy::too_many_arguments)]
pub fn trade(
    client: &AxisClient,
    direction: TradeDirection,
    kind: OrderKind,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
    orders: &Vec<u128>,
) -> (i128, i128, Option<u128>) {
    client.trade(
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
        &None,
    )
}

/// `try_trade` with a fresh nonce and no approval, returning the error code on failure
#[allow(clippy::too_many_arguments)]
pub fn try_trade(
    client: &AxisClient,
    direction: TradeDirection,
    kind: OrderKind,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
    orders: &Vec<u128>,
) -> Option<u32> {
    code(client.try_trade(
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
        &None,
    ))
}

/// Open the market for the pair: a fresh XRF holder subsidizes it with exactly the market
/// listing fee, which buys 90 days of price feeds split across the listed assets
pub fn open_market(client: &AxisClient, a: &Address, b: &Address) {
    let sponsor = actor(&client.env);
    client.subsidize(&sponsor, a, b, &MARKET_LISTING_FEE);
}

/// Open the market for the pair unless it is already open
pub fn ensure_market(client: &AxisClient, a: &Address, b: &Address) {
    if client.market(a, b).is_none() {
        open_market(client, a, b);
    }
}

/// Create a sell limit order, returning its id. Opens the market first if it does not exist
pub fn store_order(
    client: &AxisClient,
    trader: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    price: i128,
) -> u128 {
    let e = client.env.clone();
    ensure_market(client, selling, buying);
    let (sold, bought, id) = trade(
        client,
        TradeDirection::Sell,
        OrderKind::Limit,
        trader,
        amount,
        selling,
        buying,
        price,
        &no_orders(&e),
    );
    assert_eq!((sold, bought), (0, 0), "nothing to match");
    id.expect("the order must have been created")
}

/// List `asset` on the oracle and quote it at `price` as of now
pub fn list_asset(e: &Env, asset: &Address, price: i128) {
    oracle_client(e).add_asset(&Asset::Stellar(asset.clone()), &0);
    set_price(e, asset, price);
}

/// Quote `asset` at `price` as of the current ledger timestamp
pub fn set_price(e: &Env, asset: &Address, price: i128) {
    oracle_client(e).set_price(
        &Asset::Stellar(asset.clone()),
        &price,
        &e.ledger().timestamp(),
    );
}

/// Drop the oracle quote for `asset`
pub fn clear_price(e: &Env, asset: &Address) {
    oracle_client(e).clear_price(&Asset::Stellar(asset.clone()));
}

/// Move the ledger clock forward
pub fn advance(e: &Env, seconds: u64) {
    e.ledger().set_timestamp(e.ledger().timestamp() + seconds);
}

/// Move the ledger sequence forward
pub fn advance_ledgers(e: &Env, ledgers: u32) {
    e.ledger()
        .set_sequence_number(e.ledger().sequence() + ledgers);
}

/// Configure the mainnet entry TTLs
pub fn mainnet_ttls(e: &Env) {
    e.ledger().set_min_persistent_entry_ttl(MIN_PERSISTENT_TTL);
    e.ledger().set_max_entry_ttl(MAX_ENTRY_TTL);
}

/// Contract configuration wired to the mock oracle
pub fn config(e: &Env) -> Config {
    Config {
        oracle: oracle_address(e),
        market_listing_fee: MARKET_LISTING_FEE,
        safety_admin: safety_admin(e),
        min_trade_size: MIN_TRADE_SIZE,
    }
}

/// Register the Axis contract wired to the mock oracle
pub fn register_axis(e: &Env) -> Address {
    e.register(Axis, (safety_admin(e), oracle_address(e), MIN_TRADE_SIZE))
}

/// Setup a basic test environment: mock auth, oracle with XRF, trader holding XRF, issuer,
/// and two assets (USD and EUR) listed on the oracle at unit price
/// Returns: (Env, trader, issuer, usd, eur)
pub fn setup_test() -> (Env, Address, Address, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    e.ledger().set_timestamp(START_TIMESTAMP);
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);
    setup_oracle(&e, &issuer);
    list_asset(&e, &usd, UNIT_PRICE);
    list_asset(&e, &eur, UNIT_PRICE);
    fund_xrf(&e, &trader);
    log!(&e, "setup | USD: {}, EUR: {}", shorten(&usd), shorten(&eur));
    (e, trader, issuer, usd, eur)
}
