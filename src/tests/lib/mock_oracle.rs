//! Minimal stand-in for the Reflector Beam oracle used in tests.
//! Mirrors the Beam interface Axis relies on (`expires`, `lastprice`, `track`, `tracked_until`,
//! `decimals`, `fee_config`) with test-only controls to list assets, set prices, access and fees.
use crate::reflector_beam::{Asset, FeeConfig, PriceData};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Map,
    Vec,
};

/// Beam error codes reproduced by the mock
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MockOracleError {
    AssetMissing = 2,
    InvalidAmount = 7,
    AccessDenied = 102,
    InvalidRequest = 103,
}

#[contracttype]
pub enum Key {
    FeeToken,
    Decimals,
    DailyFee,
    Listed,
    Prices,
    Access,
    Calls,
}

const DAY: i128 = 86_400;

#[contract]
pub struct MockOracle;

fn listed(e: &Env) -> Map<Asset, u64> {
    e.storage()
        .instance()
        .get(&Key::Listed)
        .unwrap_or(Map::new(e))
}

fn prices(e: &Env) -> Map<Asset, PriceData> {
    e.storage()
        .instance()
        .get(&Key::Prices)
        .unwrap_or(Map::new(e))
}

fn access(e: &Env) -> Map<(Address, Asset), u64> {
    e.storage()
        .instance()
        .get(&Key::Access)
        .unwrap_or(Map::new(e))
}

#[contractimpl]
impl MockOracle {
    pub fn __constructor(e: Env, fee_token: Address, decimals: u32, daily_fee: i128) {
        e.storage().instance().set(&Key::FeeToken, &fee_token);
        e.storage().instance().set(&Key::Decimals, &decimals);
        e.storage().instance().set(&Key::DailyFee, &daily_fee);
    }

    // ---- test controls ------------------------------------------------------------------

    pub fn add_asset(e: Env, asset: Asset, expires: u64) {
        let mut assets = listed(&e);
        assets.set(asset, expires);
        e.storage().instance().set(&Key::Listed, &assets);
    }

    pub fn remove_asset(e: Env, asset: Asset) {
        let mut assets = listed(&e);
        assets.remove(asset);
        e.storage().instance().set(&Key::Listed, &assets);
    }

    pub fn set_price(e: Env, asset: Asset, price: i128, timestamp: u64) {
        let mut records = prices(&e);
        records.set(asset, PriceData { price, timestamp });
        e.storage().instance().set(&Key::Prices, &records);
    }

    pub fn clear_price(e: Env, asset: Asset) {
        let mut records = prices(&e);
        records.remove(asset);
        e.storage().instance().set(&Key::Prices, &records);
    }

    pub fn set_access(e: Env, consumer: Address, asset: Asset, until: u64) {
        let mut records = access(&e);
        records.set((consumer, asset), until);
        e.storage().instance().set(&Key::Access, &records);
    }

    /// Change the daily per-asset fee; zero reads as fees not configured
    pub fn set_daily_fee(e: Env, daily_fee: i128) {
        e.storage().instance().set(&Key::DailyFee, &daily_fee);
    }

    pub fn fee_token(e: Env) -> Address {
        e.storage().instance().get(&Key::FeeToken).unwrap()
    }

    /// Number of `lastprice` invocations so far
    pub fn calls(e: Env) -> u32 {
        e.storage().instance().get(&Key::Calls).unwrap_or(0)
    }

    // ---- Beam interface -----------------------------------------------------------------

    pub fn base(_e: Env) -> Asset {
        Asset::Other(symbol_short!("USD"))
    }

    pub fn fee_config(e: Env) -> FeeConfig {
        let daily_fee: i128 = e.storage().instance().get(&Key::DailyFee).unwrap();
        if daily_fee == 0 {
            return FeeConfig::None;
        }
        FeeConfig::Some((Self::fee_token(e), daily_fee))
    }

    pub fn decimals(e: Env) -> u32 {
        e.storage().instance().get(&Key::Decimals).unwrap()
    }

    pub fn assets(e: Env) -> Vec<Asset> {
        listed(&e).keys()
    }

    pub fn expires(e: Env, asset: Asset) -> Option<u64> {
        match listed(&e).get(asset) {
            Some(expires) => Some(expires),
            None => e.panic_with_error(MockOracleError::AssetMissing),
        }
    }

    pub fn lastprice(e: Env, caller: Address, asset: Asset) -> Option<PriceData> {
        caller.require_auth();
        let now = e.ledger().timestamp();
        let until = access(&e).get((caller, asset.clone())).unwrap_or(0);
        if !listed(&e).contains_key(asset.clone()) || until < now {
            e.panic_with_error(MockOracleError::AccessDenied);
        }
        let calls = Self::calls(e.clone()) + 1;
        e.storage().instance().set(&Key::Calls, &calls);
        prices(&e).get(asset)
    }

    pub fn track(
        e: Env,
        sponsor: Address,
        consumer: Address,
        assets: Vec<Asset>,
        amount: i128,
    ) -> Vec<u64> {
        sponsor.require_auth();
        if assets.is_empty() {
            e.panic_with_error(MockOracleError::InvalidRequest);
        }
        if amount <= 0 {
            e.panic_with_error(MockOracleError::InvalidAmount);
        }
        let daily_fee: i128 = e.storage().instance().get(&Key::DailyFee).unwrap();
        let share = amount / assets.len() as i128;
        let duration = share * DAY / daily_fee;
        if duration <= 0 {
            e.panic_with_error(MockOracleError::InvalidAmount);
        }
        let all = listed(&e);
        let mut seen: Map<Asset, bool> = Map::new(&e);
        for asset in assets.iter() {
            if !all.contains_key(asset.clone()) || seen.contains_key(asset.clone()) {
                e.panic_with_error(MockOracleError::InvalidRequest);
            }
            seen.set(asset, true);
        }
        token::Client::new(&e, &Self::fee_token(e.clone())).burn(&sponsor, &amount);
        let now = e.ledger().timestamp();
        let mut records = access(&e);
        let mut result = Vec::new(&e);
        for asset in assets.iter() {
            let key = (consumer.clone(), asset);
            let current = records.get(key.clone()).unwrap_or(0);
            let until = current.max(now) + duration as u64;
            records.set(key, until);
            result.push_back(until);
        }
        e.storage().instance().set(&Key::Access, &records);
        result
    }

    pub fn tracked_until(e: Env, consumer: Address, assets: Vec<Asset>) -> Vec<u64> {
        let records = access(&e);
        let mut result = Vec::new(&e);
        for asset in assets.iter() {
            result.push_back(records.get((consumer.clone(), asset)).unwrap_or(0));
        }
        result
    }
}
