//! A market represents an asset pair. The record is stored under its two assets in canonical (sorted) order. Markets verified against the price oracle.
use crate::errors::OrderbookError;
use crate::events;
use crate::pricing;
use crate::reflector_beam::{Asset, FeeConfig, ReflectorBeamClient};
use crate::ttl::{bump_market, bump_market_for_order, DEFAULT_LEDGER_TIME};
use soroban_sdk::{contracttype, token, Address, Env, Vec};

/// Upper bound for the oracle price decimals accepted
const MAX_ORACLE_DECIMALS: u32 = 24;
/// Upper bound for oracle decimals + token decimals, keeps `10^(sum)` within i128. An asset beyond
/// it counts as unlisted
const MAX_TOTAL_DECIMALS: u32 = 37;
/// One whole USD in `Config::min_trade_size` units, matching Stellar classic asset precision
pub const MIN_TRADE_SIZE_UNIT: i128 = 10i128.pow(7);
/// Days of price feeds the market listing fee buys until the safety admin changes it
pub const DEFAULT_LISTING_MIN_DAYS: u32 = 90;
/// Most days of price feeds the market listing fee can be set to buy
pub(crate) const MAX_LISTING_MIN_DAYS: u32 = 255;

/// Contract storage keys
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract configuration (instance)
    Config,
    /// Price oracle decimals (instance)
    OracleDecimals,
    /// Withdrawal-only mode flag (instance)
    Frozen,
    /// Market record for the canonically ordered asset pair (persistent)
    Market(Address, Address),
}

/// Contract configuration
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Account allowed to freeze the contract and change the configuration
    pub safety_admin: Address,
    /// Price oracle contract address
    pub oracle: Address,
    /// Days of price feeds a new market must buy, zero opens markets without a fee
    pub listing_min_days: u32,
    /// Amount of oracle fee tokens paid by the market creator to provision the oracle price
    /// feeds for the listed market assets
    pub market_listing_fee: i128,
    /// Minimum value a trade must sell, in USD with 7 decimals (1 USD = `MIN_TRADE_SIZE_UNIT`),
    /// zero disables the limit
    pub min_trade_size: i128,
    /// Expected average ledger close time in seconds
    pub ledger_time: u32,
}

/// Market asset descriptor
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketSide {
    /// Token contract address
    pub asset: Address,
    /// Whether the asset is quoted by the oracle (with token decimals the valuation can handle)
    pub listed: bool,
    /// Token decimals (fetched only for listed assets)
    pub decimals: u32,
}

/// Market record, stored on-chain
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Market {
    /// Base asset, the first of the pair in canonical order
    pub base: MarketSide,
    /// Quote asset, the second of the pair in canonical order
    pub quote: MarketSide,
    /// Creation timestamp
    pub created: u64,
}

impl Market {
    /// Resolve the market side for the given asset
    pub(crate) fn side(&self, asset: &Address) -> &MarketSide {
        if &self.base.asset == asset {
            &self.base
        } else {
            &self.quote
        }
    }

    /// Oracle-listed market assets in canonical order
    pub(crate) fn listed_assets(&self, e: &Env) -> Vec<Asset> {
        let mut assets = Vec::new(e);
        for side in [&self.base, &self.quote] {
            if side.listed {
                assets.push_back(Asset::Stellar(side.asset.clone()));
            }
        }
        assets
    }
}

/// Reject a negative minimum trade size, shared by the constructor and the safety admin setter
#[inline]
pub(crate) fn check_min_trade_size(e: &Env, min_trade_size: i128) {
    if min_trade_size < 0 {
        e.panic_with_error(OrderbookError::InvalidAmount);
    }
}

/// Store the contract configuration, announce it with a `config` event
pub(crate) fn set_config(e: &Env, config: &Config) {
    e.storage().instance().set(&DataKey::Config, config);
    events::emit_config(e, config);
}

/// Daily per-asset fee of an oracle. An oracle that charges no fee cannot provision markets, and
/// one whose fee overflows i128 at `MAX_LISTING_MIN_DAYS` is rejected as well
fn daily_fee(e: &Env, oracle: &ReflectorBeamClient) -> i128 {
    match oracle.fee_config() {
        FeeConfig::Some((_, fee))
            if fee > 0 && fee.checked_mul(MAX_LISTING_MIN_DAYS as i128).is_some() =>
        {
            fee
        }
        _ => e.panic_with_error(OrderbookError::InvalidOracleConfig),
    }
}

/// Validate an oracle contract. Returns its price decimals and its daily per-asset fee
pub(crate) fn check_oracle(e: &Env, oracle: &Address) -> (u32, i128) {
    let oracle = ReflectorBeamClient::new(e, oracle);
    let decimals = oracle.decimals();
    if decimals > MAX_ORACLE_DECIMALS {
        e.panic_with_error(OrderbookError::InvalidOracleConfig);
    }
    (decimals, daily_fee(e, &oracle))
}

/// Daily per-asset fee of the configured oracle
pub(crate) fn oracle_daily_fee(e: &Env) -> i128 {
    daily_fee(e, &oracle_client(e))
}

/// Market listing fee: `listing_min_days` days of the oracle's daily fee, which `daily_fee` keeps
/// within i128
pub(crate) fn listing_fee(daily_fee: i128, listing_min_days: u32) -> i128 {
    daily_fee * listing_min_days as i128
}

/// Store the oracle price decimals snapshot
pub(crate) fn set_oracle_decimals(e: &Env, decimals: u32) {
    e.storage()
        .instance()
        .set(&DataKey::OracleDecimals, &decimals);
}

/// Validate and store the contract configuration along with the oracle price decimals, deriving
/// the market listing fee from the oracle
pub(crate) fn init(e: &Env, safety_admin: Address, oracle: Address, min_trade_size: i128) {
    check_min_trade_size(e, min_trade_size);
    let (decimals, daily_fee) = check_oracle(e, &oracle);
    let config = Config {
        safety_admin,
        oracle,
        listing_min_days: DEFAULT_LISTING_MIN_DAYS,
        market_listing_fee: listing_fee(daily_fee, DEFAULT_LISTING_MIN_DAYS),
        min_trade_size,
        ledger_time: DEFAULT_LEDGER_TIME,
    };
    set_config(e, &config);
    set_oracle_decimals(e, decimals);
}

/// Contract configuration
pub(crate) fn config(e: &Env) -> Config {
    e.storage().instance().get(&DataKey::Config).unwrap()
}

/// Oracle client for the configured oracle address
pub(crate) fn oracle_client(e: &Env) -> ReflectorBeamClient<'_> {
    ReflectorBeamClient::new(e, &config(e).oracle)
}

/// Oracle price decimals
pub(crate) fn oracle_decimals(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::OracleDecimals)
        .unwrap()
}

/// Order asset pair canonically, so both trade directions map to the same market
pub(crate) fn canonical(x: &Address, y: &Address) -> (Address, Address) {
    if x <= y {
        (x.clone(), y.clone())
    } else {
        (y.clone(), x.clone())
    }
}

fn market_key(x: &Address, y: &Address) -> DataKey {
    let (base, quote) = canonical(x, y);
    DataKey::Market(base, quote)
}

/// Load market for the asset pair (any order), extending its TTL
pub(crate) fn load_market(e: &Env, x: &Address, y: &Address) -> Option<Market> {
    let key = market_key(x, y);
    let market: Option<Market> = e.storage().persistent().get(&key);
    if market.is_some() {
        bump_market(e, &key);
    }
    market
}

/// Load the market for the asset pair (any order) an order is written to, extending its TTL to
/// outlive the order: an archived market would hold up every order on it
fn load_order_market(e: &Env, x: &Address, y: &Address) -> Option<Market> {
    let key = market_key(x, y);
    let market: Option<Market> = e.storage().persistent().get(&key);
    if market.is_some() {
        bump_market_for_order(e, &key);
    }
    market
}

/// Check oracle listing and fetch token decimals for a market asset. An asset whose decimals
/// together with the oracle decimals exceed `MAX_TOTAL_DECIMALS` cannot be valued and counts as
/// unlisted, like an asset the oracle does not quote
fn resolve_side(e: &Env, oracle: &ReflectorBeamClient, asset: Address) -> MarketSide {
    //the oracle panics with AssetMissing for unknown assets
    let mut listed = oracle.try_expires(&Asset::Stellar(asset.clone())).is_ok();
    let mut decimals = 0;
    if listed {
        decimals = token::Client::new(e, &asset).decimals();
        if oracle_decimals(e).saturating_add(decimals) > MAX_TOTAL_DECIMALS {
            listed = false;
            decimals = 0;
        }
    }
    MarketSide {
        asset,
        listed,
        decimals,
    }
}

/// Reject a market with no asset quoted by the oracle: it cannot be valued
fn require_listed(e: &Env, market: &Market) {
    if !market.base.listed && !market.quote.listed {
        e.panic_with_error(OrderbookError::AssetsNotVerifiedByOracle);
    }
}

/// Re-check both market sides against the oracle, `None` if the market does not exist
pub(crate) fn refresh(e: &Env, x: &Address, y: &Address) -> Option<Market> {
    let key = market_key(x, y);
    let stored: Market = e.storage().persistent().get(&key)?;
    let oracle = ReflectorBeamClient::new(e, &config(e).oracle);
    let mut market = stored.clone();
    market.base = resolve_side(e, &oracle, stored.base.asset.clone());
    market.quote = resolve_side(e, &oracle, stored.quote.asset.clone());
    if market != stored {
        e.storage().persistent().set(&key, &market);
    }
    bump_market(e, &key);
    events::emit_refresh(e, &market.base.asset, &market.quote.asset);
    Some(market)
}

/// Load the market a `Limit` order or a changed order is written to: it must exist and have an
/// asset quoted by the oracle
pub(crate) fn load_listed_market(e: &Env, x: &Address, y: &Address) -> Market {
    match load_order_market(e, x, y) {
        Some(market) => {
            //the assets may have been delisted since the market was last checked
            require_listed(e, &market);
            market
        }
        None => e.panic_with_error(OrderbookError::AssetsNotVerifiedByOracle),
    }
}

/// Create the market for a pair that has none, provisioning the oracle feeds for the market
/// listing fee at the expense of `sponsor`; a zero fee provisions nothing. At least one asset must
/// be quoted by the oracle
fn create_market(
    e: &Env,
    sponsor: &Address,
    x: &Address,
    y: &Address,
    listing_fee: i128,
) -> Market {
    let (base, quote) = canonical(x, y);
    let oracle = oracle_client(e);
    let now = e.ledger().timestamp();
    let market = Market {
        base: resolve_side(e, &oracle, base.clone()),
        quote: resolve_side(e, &oracle, quote.clone()),
        created: now,
    };
    require_listed(e, &market);
    //purchase price feeds access for the listed assets, XRF is burned from the sponsor
    if listing_fee > 0 {
        oracle.track(
            sponsor,
            &e.current_contract_address(),
            &market.listed_assets(e),
            &listing_fee,
        );
    }
    let key = DataKey::Market(base.clone(), quote.clone());
    e.storage().persistent().set(&key, &market);
    bump_market(e, &key);
    events::emit_refresh(e, &base, &quote);
    market
}

/// Spend `amount` of the sponsor's XRF on oracle feeds access for a market. An existing market is
/// re-checked against the oracle first; a missing one is created, and the market listing fee is
/// taken out of `amount` (`InvalidAmount` when it does not cover it), so the sponsor never burns
/// more than `amount`.
/// Returns the access expiration per oracle-listed asset
pub(crate) fn fund(e: &Env, sponsor: &Address, x: &Address, y: &Address, amount: i128) -> Vec<u64> {
    let (market, subsidy) = match refresh(e, x, y) {
        Some(market) => {
            require_listed(e, &market);
            (market, amount)
        }
        None => {
            let listing_fee = config(e).market_listing_fee;
            if amount < listing_fee {
                e.panic_with_error(OrderbookError::InvalidAmount);
            }
            let market = create_market(e, sponsor, x, y, listing_fee);
            (market, amount - listing_fee)
        }
    };
    let oracle = oracle_client(e);
    let consumer = e.current_contract_address();
    let assets = market.listed_assets(e);
    //a new market funded with exactly the listing fee is already provisioned
    let access = if subsidy > 0 {
        oracle.track(sponsor, &consumer, &assets, &subsidy)
    } else {
        oracle.tracked_until(&consumer, &assets)
    };
    //with the access just bought, cache the prices the market's orders are valued against
    pricing::fetch_prices(e, &market);
    access
}
