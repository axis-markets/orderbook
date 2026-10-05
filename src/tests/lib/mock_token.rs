//! Minimal token with per-account holding limits, standing in for a classic trustline limit (the
//! test environment has no trustlines). Receiving more than the limit fails, like a full trustline.
//! It exposes no `authorized`, so Axis treats every account as able to receive it. Its decimals
//! (7 by default) can be changed to exercise high-precision tokens.
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MockTokenError {
    /// Spending more than the balance
    Balance = 10,
    /// Spending more than the allowance
    Allowance = 9,
    /// Receiving beyond the holder's limit
    LineFull = 11,
}

#[contracttype]
enum Key {
    Balance(Address),
    Allowance(Address, Address),
    Limit(Address),
    Decimals,
}

#[contract]
pub struct MockToken;

fn balance_of(e: &Env, id: &Address) -> i128 {
    e.storage()
        .persistent()
        .get(&Key::Balance(id.clone()))
        .unwrap_or(0)
}

fn spend(e: &Env, from: &Address, amount: i128) {
    let balance = balance_of(e, from);
    if balance < amount {
        e.panic_with_error(MockTokenError::Balance);
    }
    e.storage()
        .persistent()
        .set(&Key::Balance(from.clone()), &(balance - amount));
}

fn receive(e: &Env, to: &Address, amount: i128) {
    let balance = balance_of(e, to) + amount;
    let limit: Option<i128> = e.storage().persistent().get(&Key::Limit(to.clone()));
    if limit.is_some_and(|limit| balance > limit) {
        e.panic_with_error(MockTokenError::LineFull);
    }
    e.storage()
        .persistent()
        .set(&Key::Balance(to.clone()), &balance);
}

#[contractimpl]
impl MockToken {
    // ---- test controls ------------------------------------------------------------------

    /// Cap what `id` may hold
    pub fn set_limit(e: Env, id: Address, limit: i128) {
        e.storage().persistent().set(&Key::Limit(id), &limit);
    }

    /// Change the reported decimals
    pub fn set_decimals(e: Env, decimals: u32) {
        e.storage().instance().set(&Key::Decimals, &decimals);
    }

    pub fn mint(e: Env, to: Address, amount: i128) {
        receive(&e, &to, amount);
    }

    // ---- token interface ----------------------------------------------------------------

    pub fn decimals(e: Env) -> u32 {
        e.storage().instance().get(&Key::Decimals).unwrap_or(7)
    }

    pub fn balance(e: Env, id: Address) -> i128 {
        balance_of(&e, &id)
    }

    pub fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        e.storage()
            .persistent()
            .get(&Key::Allowance(from, spender))
            .unwrap_or(0)
    }

    pub fn approve(e: Env, from: Address, spender: Address, amount: i128, _live_until: u32) {
        from.require_auth();
        e.storage()
            .persistent()
            .set(&Key::Allowance(from, spender), &amount);
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        spend(&e, &from, amount);
        receive(&e, &to, amount);
    }

    pub fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        let allowance = Self::allowance(e.clone(), from.clone(), spender.clone());
        if allowance < amount {
            e.panic_with_error(MockTokenError::Allowance);
        }
        e.storage().persistent().set(
            &Key::Allowance(from.clone(), spender),
            &(allowance - amount),
        );
        spend(&e, &from, amount);
        receive(&e, &to, amount);
    }
}
