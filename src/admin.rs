use crate::errors::OrderbookError;
use crate::events;
use crate::market::{
    check_min_trade_size, check_oracle, config, listing_fee, oracle_daily_fee, set_config,
    set_oracle_decimals, DataKey, MAX_LISTING_MIN_DAYS,
};
use crate::ttl::MAX_LEDGER_TIME;
use soroban_sdk::{Address, Env};

/// Require authorization from the account allowed to operate the emergency switch
pub(crate) fn require_safety_admin(e: &Env) {
    config(e).safety_admin.require_auth();
}

/// Check whether the contract is in withdrawal-only mode
#[inline]
pub(crate) fn is_frozen(e: &Env) -> bool {
    e.storage()
        .instance()
        .get(&DataKey::Frozen)
        .unwrap_or(false)
}

/// Toggle withdrawal-only mode
pub(crate) fn set_frozen(e: &Env, blocked: bool) {
    e.storage().instance().set(&DataKey::Frozen, &blocked);
    events::emit_freeze(e, blocked);
}

/// Reject any trading activity while the contract is in withdrawal-only mode
#[inline]
pub(crate) fn require_not_frozen(e: &Env) {
    if is_frozen(e) {
        e.panic_with_error(OrderbookError::Frozen);
    }
}

/// Transfer the safety admin role over to another account
pub(crate) fn set_safety_admin(e: &Env, new_safety_admin: &Address) {
    let mut updated = config(e);
    updated.safety_admin = new_safety_admin.clone();
    set_config(e, &updated);
}

/// Point the contract at another price oracle, re-deriving the market listing fee from it. Cached
/// prices stay usable when the new oracle quotes with the same decimals
pub(crate) fn set_oracle(e: &Env, oracle: &Address) {
    let (decimals, daily_fee) = check_oracle(e, oracle);
    let mut updated = config(e);
    updated.oracle = oracle.clone();
    updated.market_listing_fee = listing_fee(daily_fee, updated.listing_min_days);
    set_config(e, &updated);
    set_oracle_decimals(e, decimals);
}

/// Update the protocol minimum trade size
pub(crate) fn set_min_trade_size(e: &Env, min_trade_size: i128) {
    check_min_trade_size(e, min_trade_size);
    let mut updated = config(e);
    updated.min_trade_size = min_trade_size;
    set_config(e, &updated);
}

/// Change the days of price feeds a new market must buy, re-deriving the market listing fee from
/// the oracle's current daily fee
pub(crate) fn set_listing_min_days(e: &Env, days: u32) {
    if days > MAX_LISTING_MIN_DAYS {
        e.panic_with_error(OrderbookError::InvalidAmount);
    }
    let mut updated = config(e);
    updated.listing_min_days = days;
    updated.market_listing_fee = listing_fee(oracle_daily_fee(e), days);
    set_config(e, &updated);
}

/// Change the ledger close time the entry lifetimes are converted into ledgers with
pub(crate) fn set_ledger_time(e: &Env, ledger_time: u32) {
    if ledger_time == 0 || ledger_time > MAX_LEDGER_TIME {
        e.panic_with_error(OrderbookError::InvalidAmount);
    }
    let mut updated = config(e);
    updated.ledger_time = ledger_time;
    set_config(e, &updated);
}
