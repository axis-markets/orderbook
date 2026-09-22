//! Argument types shared by the trading entry points.
use soroban_sdk::{contracttype, token, Address, Env, Vec};

/// A trade step in a multi-market swap path.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeStep {
    /// Asset to buy at this step
    pub asset: Address,
    /// Maker order ids to match
    pub orders: Vec<u128>,
}

/// Token allowance granted to the contract as part of the call
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approval {
    /// Token the allowance is granted on
    pub asset: Address,
    /// Absolute allowance amount
    pub amount: i128,
    /// Ledger sequence the allowance lives until
    pub live_until: u32,
}

/// New amount, price and expiration for an existing order
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderUpdate {
    /// Order id
    pub id: u128,
    /// New amount to sell, 0 removes the order
    pub amount: i128,
    /// New order price (ignored when the order is removed)
    pub price: i128,
    /// New expiration timestamp, 0 = no expiration (ignored when the order is removed)
    pub expires: u64,
}

/// Grant the contract the allowance on `approval.asset` on behalf of `trader`
pub(crate) fn approve(e: &Env, trader: &Address, approval: &Approval) {
    token::Client::new(e, &approval.asset).approve(
        trader,
        &e.current_contract_address(),
        &approval.amount,
        &approval.live_until,
    );
}
