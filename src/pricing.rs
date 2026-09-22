//! Order valuation against cached oracle prices. Only `requote` and `subsidize` talk to the
//! oracle (`fetch_prices`); `trade` and `update` value orders from the cache alone.
use crate::errors::OrderbookError;
use crate::market::{config, oracle_decimals, Market, MIN_TRADE_SIZE_UNIT};
use crate::math::{mul_div_ceil, PRECISION};
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
    /// Oracle the price was read from. A record left behind by a previous oracle is discarded
    /// rather than reused: its decimals and base currency need not match the current one
    pub oracle: Address,
}

/// Elapsed time since `timestamp`, zero for timestamps ahead of the ledger clock
fn age(now: u64, timestamp: u64) -> u64 {
    now.saturating_sub(timestamp)
}

/// Pull fresh quotes for the market's oracle-listed assets into the cache. A failed or empty
/// quote leaves the previous record in place, which keeps serving until it ages out
pub(crate) fn fetch_prices(e: &Env, market: &Market) {
    let oracle = config(e).oracle;
    let client = ReflectorBeamClient::new(e, &oracle);
    for side in [&market.a, &market.b] {
        if side.listed {
            fetch_price(e, &client, &oracle, &side.asset);
        }
    }
}

/// Cache the oracle's last price for `asset`, nothing is written when the oracle has no new data
fn fetch_price(e: &Env, client: &ReflectorBeamClient, oracle: &Address, asset: &Address) {
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
            &cache.oracle != oracle
                || cache.price != data.price
                || cache.timestamp != data.timestamp
        }
        None => true,
    };
    if changed {
        let record = PriceCache {
            price: data.price,
            timestamp: data.timestamp,
            oracle: oracle.clone(),
        };
        e.storage().temporary().set(asset, &record);
        bump_price(e, asset);
    }
}

/// Asset price in oracle base currency, read from the cache only: the oracle is never called
/// here. `None` for a missing record, one cached under a previous oracle, or one older than
/// `MAX_PRICE_AGE`
pub(crate) fn cached_price(e: &Env, asset: &Address) -> Option<i128> {
    let now = e.ledger().timestamp();
    let oracle = config(e).oracle;
    let cached: Option<PriceCache> = e.storage().temporary().get(asset);
    match cached {
        Some(cache) if cache.oracle == oracle && age(now, cache.timestamp) <= MAX_PRICE_AGE => {
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
    let selling_price = cached_price(e, selling);
    let buying_price = cached_price(e, buying);
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
        _ if !sell_side.listed && !buy_side.listed => return,
        _ => e.panic_with_error(OrderbookError::AssetPriceOracleFetchFailed),
    };
    //tokens / 10^token_decimals * price / 10^oracle_decimals >= min_trade_size / 10^size_decimals,
    //cross-multiplied. The threshold rounds up, so the configured floor is never undershot
    //`MAX_TOTAL_DECIMALS`, checked when the side is resolved, keeps the power within i128;
    //after a switch to an oracle with more decimals it holds again once `requote` re-checks
    let scale = 10i128.pow(oracle_decimals(e) + side.decimals);
    let threshold = mul_div_ceil(e, min_trade_size, scale, MIN_TRADE_SIZE_UNIT);
    let valid = match tokens.checked_mul(price) {
        Some(value) => value >= threshold,
        //the product does not fit i128, so it exceeds any threshold that does
        None => true,
    };
    if !valid {
        e.panic_with_error(OrderbookError::OrderSizeTooSmall);
    }
}
