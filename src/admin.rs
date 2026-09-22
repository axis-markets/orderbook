use crate::errors::OrderbookError;
use crate::events;
use crate::market::{
    check_min_trade_size, check_oracle, config, set_config, set_oracle_decimals, DataKey,
};
use soroban_sdk::{Address, Env};

/// Require authorization from the account allowed to operate the emergency switch
pub(crate) fn require_safety_admin(e: &Env) {
    config(e).safety_admin.require_auth();
}

/// Check whether the contract is in withdrawal-only mode
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
pub(crate) fn require_not_frozen(e: &Env) {
    if is_frozen(e) {
        e.panic_with_error(OrderbookError::Frozen);
    }
}

/// Transfer the safety admin role over to another account
pub(crate) fn set_safety_admin(e: &Env, new_safety_admin: &Address) {
    let mut updated = config(e);
    let previous = updated.safety_admin;
    updated.safety_admin = new_safety_admin.clone();
    set_config(e, &updated);
    events::emit_delegate(e, new_safety_admin.clone(), previous);
}

/// Point the contract at another price oracle, re-deriving the market listing fee from it
pub(crate) fn set_oracle(e: &Env, oracle: &Address) {
    let (decimals, market_listing_fee) = check_oracle(e, oracle);
    let mut updated = config(e);
    let previous = updated.oracle;
    updated.oracle = oracle.clone();
    updated.market_listing_fee = market_listing_fee;
    set_config(e, &updated);
    set_oracle_decimals(e, decimals);
    events::emit_oracle(e, oracle.clone(), previous);
}

/// Update the protocol minimum trade size
pub(crate) fn set_min_trade_size(e: &Env, min_trade_size: i128) {
    check_min_trade_size(e, min_trade_size);
    let mut updated = config(e);
    updated.min_trade_size = min_trade_size;
    set_config(e, &updated);
}
