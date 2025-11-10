use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

const LPH: u32 = 720; //estimated ledgers per hour
const SETTINGS: Symbol = symbol_short!("settings");
/// Price precision
pub const PRECISION: i128 = 10i128.pow(19);

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub admin: Address,
    pub fee: u32,
}

pub fn get_settings(e: &Env) -> Option<Settings> {
    e.storage().instance().get(&SETTINGS)
}

pub fn update_settings(e: &Env, settings: &Settings) {
    e.storage().instance().set(&SETTINGS, settings);
    bump_instance(&e, 10);
}

// Extend TTL for 30 days if less than X days TTL left
pub fn bump_instance(e: &Env, days_left: u32) {
    let min = LPH * 24 * days_left;
    let extend = LPH * 24 * 30;
    e.storage().instance().extend_ttl(min, extend);
}

pub fn get_fee(e: &Env) -> u32 {
    get_settings(&e).unwrap().fee
}