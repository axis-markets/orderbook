use soroban_sdk::Env;

const LPH: u32 = 720; //estimated ledgers per hour

// Extend TTL for 5 days if less than 1 days TTL left
pub fn bump_instance(e: &Env) {
    let min = LPH * 24;
    let extend = LPH * 24 * 5;
    e.storage().instance().extend_ttl(min, extend);
}