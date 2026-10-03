use crate::errors::OrderbookError;
use crate::events;
use crate::ttl::bump_order;
use soroban_sdk::xdr::{ScErrorType, ToXdr};
use soroban_sdk::{contracttype, token, Address, Env, FromVal, IntoVal, Val, Vec};

/// Trading order type - instructions to contract how to execute the trade
#[contracttype]
#[repr(i16)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderKind {
    /// Execute trade, create a limit order if not executed in full
    Limit = 1,
    /// Execute trade without creating a limit order
    Fill = 2,
    /// Execute trade, fail if it cannot be executed in full
    FillOrKill = 3,
}

/// Trade direction instructions
#[contracttype]
#[repr(i16)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TradeDirection {
    /// Sell a fixed amount of asset
    Sell = 1,
    /// Buy a fixed amount of asset
    Buy = 2,
}

/// Order properties, stored on-chain
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Order {
    /// Unique order identifier, derived from the owner and the client nonce
    pub id: u128,
    /// Selling token address
    pub selling: Address,
    /// Buying token address
    pub buying: Address,
    /// Amount left to sell
    pub amount: i128,
    /// Maker address
    pub owner: Address,
    /// Order price (`buying` per 1 `selling`, 18 decimals)
    pub price: i128,
    /// Expiration timestamp (0 = no expiration)
    pub expires: u64,
}

impl Order {
    /// Whether the order has expired by `now`; a zero `expires` never expires
    pub(crate) fn is_expired(&self, now: u64) -> bool {
        self.expires != 0 && self.expires <= now
    }
}

/// Order id: the first 16 bytes of `sha256(xdr([owner, nonce]))`
pub(crate) fn order_id(e: &Env, owner: &Address, nonce: u64) -> u128 {
    let data = (owner.clone(), nonce).to_xdr(e);
    let hash = e.crypto().sha256(&data).to_array();
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash[..16]);
    u128::from_be_bytes(id)
}

/// Stored form of an order: `[owner, selling, buying, amount, price]`, plus `expires` when the
/// order expires. Positional and without the id (the entry key) to keep the ledger entry small
#[inline]
fn encode(e: &Env, order: &Order) -> Vec<Val> {
    let mut data: Vec<Val> = Vec::new(e);
    data.push_back(order.owner.to_val());
    data.push_back(order.selling.to_val());
    data.push_back(order.buying.to_val());
    data.push_back(order.amount.into_val(e));
    data.push_back(order.price.into_val(e));
    if order.expires != 0 {
        data.push_back(order.expires.into_val(e));
    }
    data
}

/// Rebuild an order from its stored form and its id
#[inline]
fn decode(e: &Env, id: u128, data: Vec<Val>) -> Order {
    let field = |i: u32| data.get_unchecked(i);
    Order {
        id,
        owner: Address::from_val(e, &field(0)),
        selling: Address::from_val(e, &field(1)),
        buying: Address::from_val(e, &field(2)),
        amount: i128::from_val(e, &field(3)),
        price: i128::from_val(e, &field(4)),
        expires: if data.len() > 5 {
            u64::from_val(e, &field(5))
        } else {
            0
        },
    }
}

/// Write an order under its id
#[inline]
fn write_order(e: &Env, order: &Order) {
    e.storage().persistent().set(&order.id, &encode(e, order));
}

/// Load a stored order, expired or not
#[inline]
pub(crate) fn load_order(e: &Env, id: u128) -> Option<Order> {
    let data: Vec<Val> = e.storage().persistent().get(&id)?;
    Some(decode(e, id, data))
}

/// Load an order that has not expired: an expired order is treated as gone everywhere
/// except `update`, which can revive it or remove its entry
#[inline]
pub(crate) fn load_live_order(e: &Env, id: u128) -> Option<Order> {
    load_order(e, id).filter(|order| !order.is_expired(e.ledger().timestamp()))
}

/// Reject an id held by a live order; an expired order frees its id.
/// Returns whether an expired order still holds the entry
#[inline]
pub(crate) fn require_free(e: &Env, id: u128) -> bool {
    match load_order(e, id) {
        Some(order) if !order.is_expired(e.ledger().timestamp()) => {
            e.panic_with_error(OrderbookError::OrderExists)
        }
        stale => stale.is_some(),
    }
}

/// Reject an expiration timestamp that is not in the future; zero means no expiration
#[inline]
pub(crate) fn check_expires(e: &Env, expires: u64) {
    if expires != 0 && expires <= e.ledger().timestamp() {
        e.panic_with_error(OrderbookError::InvalidExpiration);
    }
}

/// Create and store a new order
#[allow(clippy::too_many_arguments)]
pub(crate) fn store_order(
    e: &Env,
    id: u128,
    owner: Address,
    amount: i128,
    selling: Address,
    buying: Address,
    price: i128,
    expires: u64,
) -> u128 {
    let replaced = require_free(e, id);
    let order = Order {
        id,
        owner,
        amount,
        selling,
        buying,
        price,
        expires,
    };
    write_order(e, &order);
    //a fresh entry starts at the network minimum lifetime, an overwritten one keeps the old TTL
    if replaced {
        bump_order(e, id, expires);
    }
    events::emit_new(e, &order);
    id
}

/// Store the amount left after a fill, dropping the order once nothing is left.
/// No event of its own: the trade event carries the amount left
pub(crate) fn apply_change(e: &Env, order: &mut Order, left: i128) {
    if left <= 0 {
        e.storage().persistent().remove(&order.id);
        return;
    }
    order.amount = left;
    write_order(e, order);
}

/// Remove an order, emitting `mod` with a zero amount
#[inline]
pub(crate) fn remove_with_mod(e: &Env, order: &Order) {
    e.storage().persistent().remove(&order.id);
    events::emit_mod(e, order.id, order.price, 0, order.expires);
}

/// Change the amount, the price and the expiration in place, emitting `mod`
pub(crate) fn modify(e: &Env, order: &mut Order, amount: i128, price: i128, expires: u64) {
    order.amount = amount;
    order.price = price;
    order.expires = expires;
    write_order(e, order);
    bump_order(e, order.id, expires);
    events::emit_mod(e, order.id, price, amount, expires);
}

/// Token balance, zero when the balance cannot be read (a missing classic trustline)
#[inline]
pub(crate) fn balance_of(e: &Env, asset: &Address, who: &Address) -> i128 {
    match token::Client::new(e, asset).try_balance(who) {
        Ok(Ok(balance)) => balance,
        _ => 0,
    }
}

/// Allowance granted by `owner` to this contract
#[inline]
pub(crate) fn allowance_of(e: &Env, asset: &Address, owner: &Address) -> i128 {
    token::Client::new(e, asset).allowance(owner, &e.current_contract_address())
}

/// Backing available for the owner's orders in `asset`: the lower of balance and allowance
#[inline]
pub(crate) fn backing(e: &Env, asset: &Address, owner: &Address) -> i128 {
    balance_of(e, asset, owner)
        .min(allowance_of(e, asset, owner))
        .max(0)
}

/// Require `owner` to hold `amount` of `asset` and to have granted at least that allowance
pub(crate) fn require_backed(e: &Env, owner: &Address, asset: &Address, amount: i128) {
    if balance_of(e, asset, owner) < amount {
        e.panic_with_error(OrderbookError::InsufficientBalance);
    }
    if allowance_of(e, asset, owner) < amount {
        e.panic_with_error(OrderbookError::InsufficientAllowance);
    }
}

/// Check `destination` can receive `asset`. A Stellar asset answers `authorized` and fails it for a
/// missing trustline; a custom token is assumed to accept
pub(crate) fn can_receive(e: &Env, asset: &Address, destination: &Address) -> bool {
    match token::StellarAssetClient::new(e, asset).try_authorized(destination) {
        Ok(Ok(authorized)) => authorized,
        Err(Ok(err)) if err.is_type(ScErrorType::Contract) => false,
        _ => true,
    }
}

/// Require `destination` to be able to receive `asset` (`CannotReceive`)
#[inline]
pub(crate) fn require_can_receive(e: &Env, asset: &Address, destination: &Address) {
    if !can_receive(e, asset, destination) {
        e.panic_with_error(OrderbookError::CannotReceive);
    }
}
