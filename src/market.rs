//! A market represents an asset pair. The record is stored under its two assets in canonical (sorted) order. Markets verified against the price oracle.
use crate::errors::OrderbookError;
use crate::pricing;
use crate::reflector_beam::{Asset, FeeConfig, ReflectorBeamClient};
use crate::ttl::bump_market;
use soroban_sdk::{contracttype, token, Address, Env, Vec};

/// Upper bound for the oracle price decimals accepted
const MAX_ORACLE_DECIMALS: u32 = 30;
/// Upper bound for oracle decimals + token decimals, keeps `10^(sum)` within i128
const MAX_TOTAL_DECIMALS: u32 = 37;
/// One whole USD in `Config::min_trade_size` units, matching Stellar classic asset precision
pub const MIN_TRADE_SIZE_UNIT: i128 = 10i128.pow(7);
/// Days of price feeds the market listing fee buys: `market_listing_fee = daily_fee * 90`
pub const LISTING_FEE_DAYS: i128 = 90;

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

/// Contract configuration. The safety admin, the oracle and the minimum trade size are supplied
/// at deployment; the market listing fee is derived from the oracle whenever the oracle is set
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Account allowed to freeze the contract and change the minimum trade size
    pub safety_admin: Address,
    /// Reflector Beam price oracle contract address
    pub oracle: Address,
    /// Amount of XRF stroops burned by the market creator to provision the oracle price
    /// feeds for the listed market assets: the oracle's daily fee times `LISTING_FEE_DAYS`,
    /// computed when the oracle is set
    pub market_listing_fee: i128,
    /// Minimum value a trade must sell, in USD with 7 decimals (1 USD = `MIN_TRADE_SIZE_UNIT`),
    /// zero disables the limit
    pub min_trade_size: i128,
}

/// Market asset descriptor
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketSide {
    /// Token contract address
    pub asset: Address,
    /// Whether the asset is quoted by the oracle
    pub listed: bool,
    /// Token decimals (fetched only for listed assets)
    pub decimals: u32,
}

/// Market record, stored on-chain
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Market {
    /// First asset (canonical order)
    pub a: MarketSide,
    /// Second asset (canonical order)
    pub b: MarketSide,
    /// Creation timestamp
    pub created: u64,
    /// Timestamp of the last check of both sides against the price oracle
    pub checked: u64,
}

impl Market {
    /// Resolve the market side for the given asset
    pub(crate) fn side(&self, asset: &Address) -> &MarketSide {
        if &self.a.asset == asset {
            &self.a
        } else {
            &self.b
        }
    }

    /// Oracle-listed market assets in canonical order
    pub(crate) fn listed_assets(&self, e: &Env) -> Vec<Asset> {
        let mut assets = Vec::new(e);
        for side in [&self.a, &self.b] {
            if side.listed {
                assets.push_back(Asset::Stellar(side.asset.clone()));
            }
        }
        assets
    }
}

/// Reject a negative minimum trade size, shared by the constructor and the safety admin setter
pub(crate) fn check_min_trade_size(e: &Env, min_trade_size: i128) {
    if min_trade_size < 0 {
        e.panic_with_error(OrderbookError::InvalidAmount);
    }
}

/// Store the contract configuration, overwriting the previous record
pub(crate) fn set_config(e: &Env, config: &Config) {
    e.storage().instance().set(&DataKey::Config, config);
}

/// Validate an oracle contract. Returns its price decimals and the market listing fee derived
/// from its daily fee (`daily_fee * LISTING_FEE_DAYS`); an oracle that charges no fee cannot
/// provision markets
pub(crate) fn check_oracle(e: &Env, oracle: &Address) -> (u32, i128) {
    let oracle = ReflectorBeamClient::new(e, oracle);
    let decimals = oracle.decimals();
    if decimals > MAX_ORACLE_DECIMALS {
        e.panic_with_error(OrderbookError::InvalidOracleConfig);
    }
    let listing_fee = match oracle.fee_config() {
        FeeConfig::Some((_, daily_fee)) if daily_fee > 0 => daily_fee.checked_mul(LISTING_FEE_DAYS),
        _ => None,
    };
    match listing_fee {
        Some(listing_fee) => (decimals, listing_fee),
        None => e.panic_with_error(OrderbookError::InvalidOracleConfig),
    }
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
    let (decimals, market_listing_fee) = check_oracle(e, &oracle);
    let config = Config {
        safety_admin,
        oracle,
        market_listing_fee,
        min_trade_size,
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
    let (a, b) = canonical(x, y);
    DataKey::Market(a, b)
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

/// Check oracle listing and fetch token decimals for a market asset
fn resolve_side(e: &Env, oracle: &ReflectorBeamClient, asset: Address) -> MarketSide {
    //the oracle panics with AssetMissing for unknown assets
    let listed = oracle.try_expires(&Asset::Stellar(asset.clone())).is_ok();
    let mut decimals = 0;
    if listed {
        decimals = token::Client::new(e, &asset).decimals();
        if oracle_decimals(e) + decimals > MAX_TOTAL_DECIMALS {
            e.panic_with_error(OrderbookError::Overflow);
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
    if !market.a.listed && !market.b.listed {
        e.panic_with_error(OrderbookError::AssetsNotVerifiedByOracle);
    }
}

/// Re-check both market sides against the oracle, `None` if the market does not exist
pub(crate) fn refresh(e: &Env, x: &Address, y: &Address) -> Option<Market> {
    let key = market_key(x, y);
    let mut market: Market = e.storage().persistent().get(&key)?;
    let oracle = ReflectorBeamClient::new(e, &config(e).oracle);
    market.a = resolve_side(e, &oracle, market.a.asset.clone());
    market.b = resolve_side(e, &oracle, market.b.asset.clone());
    market.checked = e.ledger().timestamp();
    e.storage().persistent().set(&key, &market);
    bump_market(e, &key);
    Some(market)
}

/// Load the market a `Limit` order stored on: it must exist and have an asset quoted by the oracle
pub(crate) fn load_listed_market(e: &Env, x: &Address, y: &Address) -> Market {
    match load_market(e, x, y) {
        Some(market) => {
            //the assets may have been delisted since the market was last checked
            require_listed(e, &market);
            market
        }
        None => e.panic_with_error(OrderbookError::AssetsNotVerifiedByOracle),
    }
}

/// Create the market for a pair that has none, provisioning the oracle feeds for the market
/// listing fee at the expense of `sponsor`. At least one asset must be quoted by the oracle
fn create_market(
    e: &Env,
    sponsor: &Address,
    x: &Address,
    y: &Address,
    listing_fee: i128,
) -> Market {
    let (a, b) = canonical(x, y);
    let oracle = oracle_client(e);
    let now = e.ledger().timestamp();
    let market = Market {
        a: resolve_side(e, &oracle, a.clone()),
        b: resolve_side(e, &oracle, b.clone()),
        created: now,
        checked: now,
    };
    require_listed(e, &market);
    //purchase price feeds access for the listed assets, XRF is burned from the sponsor
    oracle.track(
        sponsor,
        &e.current_contract_address(),
        &market.listed_assets(e),
        &listing_fee,
    );
    let key = DataKey::Market(a, b);
    e.storage().persistent().set(&key, &market);
    bump_market(e, &key);
    market
}

/// Spend `amount` of the sponsor's XRF on oracle feeds access for a market. An existing market is
/// re-checked against the oracle first; a missing one is created, and the market listing fee is
/// taken out of `amount` (`InvalidAmount` when it does not cover it), so the sponsor never burns
/// more than `amount`. Returns the access expiration per oracle-listed asset
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
