use crate::market::DataKey;
use crate::pricing::MAX_PRICE_AGE;
use soroban_sdk::{Address, Env};

const LPD: u32 = 17_280; //ledgers per day at 5 s per ledger

/// Extend the contract instance and code TTL to 180 days once less than 30 days are left,
/// capped by the network maximum
pub(crate) fn bump_contract(e: &Env) {
    let max_extension = e
        .ledger()
        .max_live_until_ledger()
        .saturating_sub(e.ledger().sequence());
    let extend_to = (LPD * 180).min(max_extension);
    let threshold = (LPD * 30).min(extend_to);
    e.deployer()
        .extend_ttl(e.current_contract_address(), threshold, extend_to);
}

/// Extend market entry TTL for 120 days if less than 30 days TTL left
pub(crate) fn bump_market(e: &Env, key: &DataKey) {
    let min = LPD * 30;
    let extend = LPD * 120;
    e.storage().persistent().extend_ttl(key, min, extend);
}

/// Set the cached price entry TTL to cover the period the record stays usable.
/// Called on every cache write, so the entry always outlives the price it holds
pub(crate) fn bump_price(e: &Env, asset: &Address) {
    let ttl = (MAX_PRICE_AGE / 86_400) as u32 * LPD;
    e.storage().temporary().extend_ttl(asset, ttl, ttl);
}
