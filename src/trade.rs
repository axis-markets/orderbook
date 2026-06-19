use soroban_sdk::{contracttype, Address, Env, Vec};

const LAST_TRADE_ID: &str = "lt";

/// Orderbook trade event
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trade {
    /// Unique trade id
    pub id: u64,
    /// Order id
    pub order: u64,
    /// Trader account address
    pub taker: Address,
    /// Seller account address
    pub maker: Address,
    /// Sold asset address
    pub selling: Address,
    /// Bought asset address
    pub buying: Address,
    /// Sold tokens amount
    pub sold: i128,
    /// Bought tokens amount
    pub bought: i128,
}

/// Orderbook swap event
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Swap {
    /// Unique swap id (last trade id assigned while settling the swap legs)
    pub id: u64,
    /// Trader account address
    pub trader: Address,
    /// Sold asset address
    pub selling: Address,
    /// Bought asset address
    pub buying: Address,
    /// Amount of `selling` tokens sold
    pub sold: i128,
    /// Amount of `buying` tokens received
    pub bought: i128,
}

/// A trade step in a multi-market swap path.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeStep {
    /// Asset to buy at this step
    pub asset: Address,
    /// Maker order IDs to match
    pub orders: Vec<u64>,
}

pub(crate) fn get_last_trade_id(e: &Env) -> u64 {
    e.storage().instance().get(&LAST_TRADE_ID).unwrap_or(0)
}

pub(crate) fn next_trade_id(e: &Env) -> u64 {
    let last = get_last_trade_id(&e) + 1;
    e.storage().instance().set(&LAST_TRADE_ID, &last);
    last
}
