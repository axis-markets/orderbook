//! Generic SEP-41 token for the AXIS tests, standing in for the custom tokens deployed as contracts
//! of their own rather than Stellar Asset Contracts. Besides the token interface it has an admin
//! who mints, pauses every transfer and blacklists accounts, the controls regulated tokens
//! commonly add. It answers no `authorized`, so AXIS treats every account as able to receive it:
//! a paused token or a blacklisted account only shows when a transfer runs. Allowances expire at
//! their `live_until_ledger`, approvals work while the token is paused.
//!
//! Build the wasm the AXIS tests import (`src/tests/lib/wasm/test_token.wasm`), from the
//! repository root:
//!
//! ```sh
//! stellar contract build --manifest-path test-token/Cargo.toml --optimize --out-dir src/tests/lib/wasm
//! ```
#![no_std]

use soroban_sdk::token::TokenInterface;
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env,
    MuxedAddress, String,
};

/// Ledgers in a day at 5 s per ledger
const DAY_IN_LEDGERS: u32 = 17_280;
/// Instance and balances are extended to this lifetime once less than `TTL_THRESHOLD` is left
const TTL_EXTEND_TO: u32 = 31 * DAY_IN_LEDGERS;
const TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TokenError {
    /// A negative amount
    NegativeAmount = 1,
    /// Spending more than the balance
    InsufficientBalance = 2,
    /// Spending more than the allowance
    InsufficientAllowance = 3,
    /// A non-zero allowance expiring before the current ledger
    InvalidExpiration = 4,
    /// Transfers are paused
    Paused = 5,
    /// An account taking part is blacklisted
    Blacklisted = 6,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// Instance: the account that mints, pauses and blacklists
    Admin,
    /// Instance: `Metadata`
    Metadata,
    /// Instance: present while transfers are paused
    Paused,
    /// Persistent: balance of an account
    Balance(Address),
    /// Persistent: present while the account is blacklisted
    Blacklisted(Address),
    /// Temporary: `AllowanceValue` of `(from, spender)`
    Allowance(Address, Address),
}

#[contracttype]
#[derive(Clone)]
struct Metadata {
    decimals: u32,
    name: String,
    symbol: String,
}

#[contracttype]
#[derive(Clone)]
struct AllowanceValue {
    amount: i128,
    live_until_ledger: u32,
}

#[contractevent(data_format = "single-value")]
struct Transfer {
    #[topic]
    from: Address,
    #[topic]
    to: Address,
    amount: i128,
}

/// `transfer` to a muxed address
#[contractevent(topics = ["transfer"])]
struct TransferMuxed {
    #[topic]
    from: Address,
    #[topic]
    to: Address,
    amount: i128,
    to_muxed_id: u64,
}

#[contractevent(data_format = "vec")]
struct Approve {
    #[topic]
    from: Address,
    #[topic]
    spender: Address,
    amount: i128,
    live_until_ledger: u32,
}

#[contractevent(data_format = "single-value")]
struct Mint {
    #[topic]
    to: Address,
    amount: i128,
}

#[contractevent(data_format = "single-value")]
struct Burn {
    #[topic]
    from: Address,
    amount: i128,
}

#[contract]
pub struct TestToken;

fn bump_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn admin(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Admin).unwrap()
}

fn require_non_negative(e: &Env, amount: i128) {
    if amount < 0 {
        e.panic_with_error(TokenError::NegativeAmount);
    }
}

fn require_not_paused(e: &Env) {
    if e.storage().instance().has(&DataKey::Paused) {
        e.panic_with_error(TokenError::Paused);
    }
}

fn is_blacklisted(e: &Env, account: &Address) -> bool {
    e.storage()
        .persistent()
        .has(&DataKey::Blacklisted(account.clone()))
}

fn require_not_blacklisted(e: &Env, accounts: &[&Address]) {
    if accounts.iter().any(|account| is_blacklisted(e, account)) {
        e.panic_with_error(TokenError::Blacklisted);
    }
}

fn read_balance(e: &Env, account: &Address) -> i128 {
    e.storage()
        .persistent()
        .get(&DataKey::Balance(account.clone()))
        .unwrap_or(0)
}

fn write_balance(e: &Env, account: &Address, amount: i128) {
    let key = DataKey::Balance(account.clone());
    e.storage().persistent().set(&key, &amount);
    e.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn spend_balance(e: &Env, account: &Address, amount: i128) {
    let balance = read_balance(e, account);
    if balance < amount {
        e.panic_with_error(TokenError::InsufficientBalance);
    }
    write_balance(e, account, balance - amount);
}

fn receive_balance(e: &Env, account: &Address, amount: i128) {
    write_balance(e, account, read_balance(e, account) + amount);
}

/// Allowance of `spender` on `from`, zero once expired
fn read_allowance(e: &Env, from: &Address, spender: &Address) -> AllowanceValue {
    let key = DataKey::Allowance(from.clone(), spender.clone());
    match e.storage().temporary().get::<_, AllowanceValue>(&key) {
        Some(allowance) if allowance.live_until_ledger >= e.ledger().sequence() => allowance,
        _ => AllowanceValue {
            amount: 0,
            live_until_ledger: 0,
        },
    }
}

fn write_allowance(e: &Env, from: &Address, spender: &Address, amount: i128, live_until: u32) {
    let key = DataKey::Allowance(from.clone(), spender.clone());
    if amount == 0 {
        e.storage().temporary().remove(&key);
        return;
    }
    let ledger = e.ledger().sequence();
    if live_until < ledger {
        e.panic_with_error(TokenError::InvalidExpiration);
    }
    let value = AllowanceValue {
        amount,
        live_until_ledger: live_until,
    };
    e.storage().temporary().set(&key, &value);
    let live_for = live_until - ledger;
    e.storage().temporary().extend_ttl(&key, live_for, live_for);
}

fn spend_allowance(e: &Env, from: &Address, spender: &Address, amount: i128) {
    let allowance = read_allowance(e, from, spender);
    if allowance.amount < amount {
        e.panic_with_error(TokenError::InsufficientAllowance);
    }
    if amount > 0 {
        let key = DataKey::Allowance(from.clone(), spender.clone());
        let value = AllowanceValue {
            amount: allowance.amount - amount,
            live_until_ledger: allowance.live_until_ledger,
        };
        e.storage().temporary().set(&key, &value);
    }
}

fn move_balance(e: &Env, from: &Address, to: &Address, amount: i128) {
    spend_balance(e, from, amount);
    receive_balance(e, to, amount);
}

#[contractimpl]
impl TestToken {
    pub fn __constructor(e: Env, admin: Address, decimals: u32, name: String, symbol: String) {
        e.storage().instance().set(&DataKey::Admin, &admin);
        let metadata = Metadata {
            decimals,
            name,
            symbol,
        };
        e.storage().instance().set(&DataKey::Metadata, &metadata);
    }

    /// Mint `amount` to `to` (admin)
    pub fn mint(e: Env, to: Address, amount: i128) {
        admin(&e).require_auth();
        require_non_negative(&e, amount);
        require_not_paused(&e);
        require_not_blacklisted(&e, &[&to]);
        bump_instance(&e);
        receive_balance(&e, &to, amount);
        Mint { to, amount }.publish(&e);
    }

    /// Pause or resume every transfer, mint and burn (admin); approvals and reads keep working
    pub fn set_paused(e: Env, paused: bool) {
        admin(&e).require_auth();
        bump_instance(&e);
        if paused {
            e.storage().instance().set(&DataKey::Paused, &());
        } else {
            e.storage().instance().remove(&DataKey::Paused);
        }
    }

    /// Whether transfers are paused
    pub fn paused(e: Env) -> bool {
        e.storage().instance().has(&DataKey::Paused)
    }

    /// Bar `account` from sending, receiving, spending and approving, or lift the bar (admin)
    pub fn set_blacklisted(e: Env, account: Address, blacklisted: bool) {
        admin(&e).require_auth();
        bump_instance(&e);
        let key = DataKey::Blacklisted(account);
        if blacklisted {
            e.storage().persistent().set(&key, &());
        } else {
            e.storage().persistent().remove(&key);
        }
    }

    /// Whether `account` is blacklisted
    pub fn blacklisted(e: Env, account: Address) -> bool {
        is_blacklisted(&e, &account)
    }
}

#[contractimpl(contracttrait)]
impl TokenInterface for TestToken {
    fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        read_allowance(&e, &from, &spender).amount
    }

    fn approve(e: Env, from: Address, spender: Address, amount: i128, live_until_ledger: u32) {
        from.require_auth();
        require_non_negative(&e, amount);
        require_not_blacklisted(&e, &[&from, &spender]);
        bump_instance(&e);
        write_allowance(&e, &from, &spender, amount, live_until_ledger);
        Approve {
            from,
            spender,
            amount,
            live_until_ledger,
        }
        .publish(&e);
    }

    fn balance(e: Env, id: Address) -> i128 {
        read_balance(&e, &id)
    }

    fn transfer(e: Env, from: Address, to: MuxedAddress, amount: i128) {
        from.require_auth();
        require_non_negative(&e, amount);
        require_not_paused(&e);
        let to_address = to.address();
        require_not_blacklisted(&e, &[&from, &to_address]);
        bump_instance(&e);
        move_balance(&e, &from, &to_address, amount);
        match to.id() {
            Some(to_muxed_id) => TransferMuxed {
                from,
                to: to_address,
                amount,
                to_muxed_id,
            }
            .publish(&e),
            None => Transfer {
                from,
                to: to_address,
                amount,
            }
            .publish(&e),
        }
    }

    fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        require_non_negative(&e, amount);
        require_not_paused(&e);
        require_not_blacklisted(&e, &[&spender, &from, &to]);
        bump_instance(&e);
        spend_allowance(&e, &from, &spender, amount);
        move_balance(&e, &from, &to, amount);
        Transfer { from, to, amount }.publish(&e);
    }

    fn burn(e: Env, from: Address, amount: i128) {
        from.require_auth();
        require_non_negative(&e, amount);
        require_not_paused(&e);
        require_not_blacklisted(&e, &[&from]);
        bump_instance(&e);
        spend_balance(&e, &from, amount);
        Burn { from, amount }.publish(&e);
    }

    fn burn_from(e: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        require_non_negative(&e, amount);
        require_not_paused(&e);
        require_not_blacklisted(&e, &[&spender, &from]);
        bump_instance(&e);
        spend_allowance(&e, &from, &spender, amount);
        spend_balance(&e, &from, amount);
        Burn { from, amount }.publish(&e);
    }

    fn decimals(e: Env) -> u32 {
        Self::metadata(&e).decimals
    }

    fn name(e: Env) -> String {
        Self::metadata(&e).name
    }

    fn symbol(e: Env) -> String {
        Self::metadata(&e).symbol
    }
}

impl TestToken {
    fn metadata(e: &Env) -> Metadata {
        e.storage().instance().get(&DataKey::Metadata).unwrap()
    }
}
