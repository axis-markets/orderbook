use crate::market::DataKey;
use crate::pricing::MAX_PRICE_AGE;
use soroban_sdk::{Address, Env};

const LPD: u32 = 17_280; //ledgers per day at 5 s per ledger
/// Approximate ledger close time (in seconds)
const LEDGER_TIME: u64 = 5;
/// Contract lifetime a regular call keeps: it tops the contract up only when keepers let it run low
pub(crate) const USER_BUMP_DAYS: u32 = 3;
/// Contract lifetime `keepalive` extends to
pub(crate) const KEEPALIVE_BUMP_DAYS: u32 = 180;
/// Order entry lifetime granted by `update`
const ORDER_TTL_DAYS: u32 = 120;

/// Extend the contract instance and code TTL to `days` once less than `days` are left, capped by
/// the network maximum
pub(crate) fn bump_contract(e: &Env, days: u32) {
    let max_extension = e
        .ledger()
        .max_live_until_ledger()
        .saturating_sub(e.ledger().sequence());
    let extend_to = (LPD * days).min(max_extension);
    e.deployer()
        .extend_ttl(e.current_contract_address(), extend_to, extend_to);
}

/// Extend market entry TTL for 120 days if less than 30 days TTL left
pub(crate) fn bump_market(e: &Env, key: &DataKey) {
    let min = LPD * 30;
    let extend = LPD * 120;
    e.storage().persistent().extend_ttl(key, min, extend);
}

/// Extend an updated (or overwritten) order entry to cover its expiration plus a day, or 120 days for an order
/// that does not expire (also the cap). Never shortens a longer TTL
pub(crate) fn bump_order(e: &Env, id: u128, expires: u64) {
    let max = LPD * ORDER_TTL_DAYS;
    let extend_to = if expires == 0 {
        max
    } else {
        let left = expires.saturating_sub(e.ledger().timestamp());
        let ledgers = left.div_ceil(LEDGER_TIME) + LPD as u64;
        ledgers.min(max as u64) as u32
    };
    e.storage()
        .persistent()
        .extend_ttl(&id, extend_to, extend_to);
}

/// Set the cached price entry TTL to cover the period the record stays usable.
/// Called on every cache write, so the entry always outlives the price it holds
pub(crate) fn bump_price(e: &Env, asset: &Address) {
    let ttl = (MAX_PRICE_AGE / 86_400) as u32 * LPD;
    e.storage().temporary().extend_ttl(asset, ttl, ttl);
}
