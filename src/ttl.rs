use crate::market::{config, DataKey};
use crate::pricing::MAX_PRICE_AGE;
use soroban_sdk::{Address, Env};

/// Seconds in a day
const DAY: u64 = 86_400;
/// Ledger close time the contract starts with (in seconds)
pub(crate) const DEFAULT_LEDGER_TIME: u32 = 5;
/// Longest ledger close time the safety admin can set (in seconds)
pub(crate) const MAX_LEDGER_TIME: u32 = 20;
/// Contract lifetime a regular call keeps: it tops the contract up only when keepers let it run low.
/// Keepers extend the instance and code with the `ExtendFootprintTTL` operation, no entry point needed
pub(crate) const USER_BUMP_DAYS: u32 = 3;
/// Order entry lifetime granted by `update`
const ORDER_TTL_DAYS: u32 = 120;

/// Configured ledger close time (in seconds)
fn ledger_time(e: &Env) -> u64 {
    config(e).ledger_time as u64
}

/// Ledgers closing within `seconds` at `ledger_time` seconds per ledger, rounded up
fn ledgers(seconds: u64, ledger_time: u64) -> u32 {
    seconds.div_ceil(ledger_time).min(u32::MAX as u64) as u32
}

/// Longest extension the network allows from the current ledger
fn max_extension(e: &Env) -> u32 {
    e.ledger()
        .max_live_until_ledger()
        .saturating_sub(e.ledger().sequence())
}

/// Extend the contract instance and code TTL to `days` once less than `days` are left, capped by
/// the network maximum
pub(crate) fn bump_contract(e: &Env, days: u32) {
    let extend_to = ledgers(days as u64 * DAY, ledger_time(e)).min(max_extension(e));
    e.deployer()
        .extend_ttl(e.current_contract_address(), extend_to, extend_to);
}

/// Extend market entry TTL for 120 days if less than 30 days TTL left
pub(crate) fn bump_market(e: &Env, key: &DataKey) {
    let ledger_time = ledger_time(e);
    let min = ledgers(30 * DAY, ledger_time);
    let extend = ledgers(120 * DAY, ledger_time);
    e.storage().persistent().extend_ttl(key, min, extend);
}

/// Extend the entry of a market an order is written to so it outlives the order
pub(crate) fn bump_market_for_order(e: &Env, key: &DataKey) {
    let ledger_time = ledger_time(e);
    let min = ledgers(ORDER_TTL_DAYS as u64 * DAY, ledger_time);
    let extend = ledgers((ORDER_TTL_DAYS + 1) as u64 * DAY, ledger_time);
    e.storage().persistent().extend_ttl(key, min, extend);
}

/// Extend an updated (or overwritten) order entry to cover its expiration plus a day, or 120 days for an order
/// that does not expire (also the cap). Never shortens a longer TTL
pub(crate) fn bump_order(e: &Env, id: u128, expires: u64) {
    let ledger_time = ledger_time(e);
    let max = ledgers(ORDER_TTL_DAYS as u64 * DAY, ledger_time);
    let extend_to = if expires == 0 {
        max
    } else {
        let left = expires.saturating_sub(e.ledger().timestamp());
        ledgers(left.saturating_add(DAY), ledger_time).min(max)
    };
    e.storage()
        .persistent()
        .extend_ttl(&id, extend_to, extend_to);
}

/// Set the cached price entry TTL to cover the period the record stays usable.
/// Called on every cache write, so the entry always outlives the price it holds
pub(crate) fn bump_price(e: &Env, asset: &Address) {
    //a temporary entry cannot be extended past the network maximum
    let ttl = ledgers(MAX_PRICE_AGE, ledger_time(e)).min(max_extension(e));
    e.storage().temporary().extend_ttl(asset, ttl, ttl);
}
