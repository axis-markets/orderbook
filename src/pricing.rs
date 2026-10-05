//! Order valuation against cached oracle prices. Only `requote` and `subsidize` talk to the
//! oracle (`fetch_prices`); `trade` and `update` value orders from the cache alone.
use crate::errors::OrderbookError;
use crate::market::{config, oracle_decimals, Market, MIN_TRADE_SIZE_UNIT};
use crate::math::{mul_div_ceil, product_reaches, PRECISION};
use crate::order::TradeDirection;
use crate::reflector_beam::{Asset, ReflectorBeamClient};
use crate::ttl::bump_price;
use soroban_sdk::{contracttype, Address, Env};

/// Maximum age of a price record still considered safe for order valuation (in seconds)
pub const MAX_PRICE_AGE: u64 = 72 * 3600;

/// Cached oracle price, kept in temporary storage under the asset address
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceCache {
    /// Price in oracle decimals
    pub price: i128,
    /// Price record timestamp (in seconds)
    pub timestamp: u64,
    /// Price decimals of the oracle the price was read from
    pub decimals: u32,
}

/// Elapsed time since `timestamp`, zero for timestamps ahead of the ledger clock
fn age(now: u64, timestamp: u64) -> u64 {
    now.saturating_sub(timestamp)
}

/// Pull fresh quotes for the market's oracle-listed assets into the cache. A failed or empty
/// quote leaves the previous record in place, which keeps serving until it ages out
pub(crate) fn fetch_prices(e: &Env, market: &Market) {
    let client = ReflectorBeamClient::new(e, &config(e).oracle);
    let decimals = oracle_decimals(e);
    for side in [&market.base, &market.quote] {
        if side.listed {
            fetch_price(e, &client, decimals, &side.asset);
        }
    }
}

/// Cache the oracle's last price for `asset`, nothing is written when the oracle has no new data
fn fetch_price(e: &Env, client: &ReflectorBeamClient, decimals: u32, asset: &Address) {
    let quote = client.try_lastprice(
        &e.current_contract_address(),
        &Asset::Stellar(asset.clone()),
    );
    //the oracle is unreachable, out of access or has no quote: keep the last known price
    let data = match quote {
        Ok(Ok(Some(data))) if data.price > 0 => data,
        _ => return,
    };
    let cached: Option<PriceCache> = e.storage().temporary().get(asset);
    let changed = match cached {
        Some(cache) => {
            cache.decimals != decimals
                || cache.price != data.price
                || cache.timestamp != data.timestamp
        }
        None => true,
    };
    if changed {
        let record = PriceCache {
            price: data.price,
            timestamp: data.timestamp,
            decimals,
        };
        e.storage().temporary().set(asset, &record);
        bump_price(e, asset);
    }
}

/// Asset price in oracle base currency, read from the cache only: the oracle is never called
/// here. `None` for a missing record, one cached in other decimals than `decimals` (the current
/// oracle's), or one older than `MAX_PRICE_AGE`
fn cached_price(e: &Env, asset: &Address, decimals: u32) -> Option<i128> {
    let now = e.ledger().timestamp();
    let cached: Option<PriceCache> = e.storage().temporary().get(asset);
    match cached {
        Some(cache) if cache.decimals == decimals && age(now, cache.timestamp) <= MAX_PRICE_AGE => {
            Some(cache.price)
        }
        _ => None,
    }
}

/// Ensure the trade sells at least `Config::min_trade_size` worth of tokens.
/// Valued on the selling asset when it is oracle-listed and has a usable cached price, otherwise
/// on the equivalent buying amount derived from the trade price. A zero minimum disables the
/// check.
pub(crate) fn enforce_min_order_value(
    e: &Env,
    market: &Market,
    direction: &TradeDirection,
    amount: i128,
    price: i128,
    selling: &Address,
    buying: &Address,
) {
    let min_trade_size = config(e).min_trade_size;
    if min_trade_size == 0 {
        return;
    }
    //both cache entries are read regardless of the listing flags and the price age, so the
    //transaction footprint does not change when either moves between simulation and execution
    let decimals = oracle_decimals(e);
    let selling_price = cached_price(e, selling, decimals);
    let buying_price = cached_price(e, buying, decimals);
    let counter = mul_div_ceil(e, amount, price, PRECISION);
    let (selling_amount, buying_amount) = match direction {
        TradeDirection::Sell => (amount, counter),
        TradeDirection::Buy => (counter, amount),
    };
    let sell_side = market.side(selling);
    let buy_side = market.side(buying);
    let (tokens, side, price) = match (selling_price, buying_price) {
        (Some(price), _) if sell_side.listed => (selling_amount, sell_side, price),
        (_, Some(price)) if buy_side.listed => (buying_amount, buy_side, price),
        //callers load the market with a listed side, which has no usable price here
        _ => e.panic_with_error(OrderbookError::AssetPriceOracleFetchFailed),
    };
    //tokens / 10^token_decimals * price / 10^oracle_decimals >= min_trade_size / 10^size_decimals,
    //cross-multiplied. The threshold rounds up, so the configured floor is never undershot and
    //the comparison stays exact however large either side gets
    let scale = 10i128.pow(decimals + side.decimals);
    if !product_reaches(e, tokens, price, min_trade_size, scale, MIN_TRADE_SIZE_UNIT) {
        e.panic_with_error(OrderbookError::OrderSizeTooSmall);
    }
}
