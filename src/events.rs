use crate::order::Order;
use soroban_sdk::{contractevent, Address, Env};

#[contractevent(topics = ["trade"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeEvent {
    /// Sold asset address
    #[topic]
    pub selling: Address,
    /// Bought asset address
    #[topic]
    pub buying: Address,
    /// Order id
    pub order: u128,
    /// Trader account address
    pub taker: Address,
    /// Seller account address
    pub maker: Address,
    /// Sold tokens amount
    pub sold: i128,
    /// Bought tokens amount
    pub bought: i128,
    /// Order amount left
    pub left: i128,
}

#[contractevent(topics = ["swap"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapEvent {
    /// Asset sold by the trader
    #[topic]
    pub selling: Address,
    /// Asset received by the trader
    #[topic]
    pub buying: Address,
    /// Trader account address
    pub trader: Address,
    /// Amount of `selling` tokens sold
    pub sold: i128,
    /// Amount of `buying` tokens received
    pub bought: i128,
}

#[contractevent(topics = ["new"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderCreatedEvent {
    /// Selling asset address
    #[topic]
    pub selling: Address,
    /// Buying asset address
    #[topic]
    pub buying: Address,
    /// Unique order identifier
    pub id: u128,
    /// Maker address
    pub owner: Address,
    /// Order price
    pub price: i128,
    /// Current amount
    pub amount: i128,
    /// Expiration timestamp (0 = no expiration)
    pub expires: u64,
}

#[contractevent(topics = ["mod"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderUpdatedEvent {
    /// Unique order identifier
    pub id: u128,
    /// Order price
    pub price: i128,
    /// Current amount (0 = removed)
    pub amount: i128,
    /// Expiration timestamp (0 = no expiration)
    pub expires: u64,
}

#[contractevent(topics = ["freeze"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezeEvent {
    /// Whether contract trading is frozen after the call
    pub frozen: bool,
}

#[contractevent(topics = ["delegate"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegateEvent {
    /// Account that received the safety admin role
    #[topic]
    pub admin: Address,
    /// Account that held the role before the call
    pub previous: Address,
}

#[contractevent(topics = ["oracle"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleEvent {
    /// Price oracle the contract now reads
    #[topic]
    pub oracle: Address,
    /// Oracle used before the call
    pub previous: Address,
}

pub(crate) fn emit_delegate(e: &Env, admin: Address, previous: Address) {
    DelegateEvent { admin, previous }.publish(e);
}

pub(crate) fn emit_oracle(e: &Env, oracle: Address, previous: Address) {
    OracleEvent { oracle, previous }.publish(e);
}

pub(crate) fn emit_freeze(e: &Env, frozen: bool) {
    FreezeEvent { frozen }.publish(e);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_trade(
    e: &Env,
    selling: &Address,
    buying: &Address,
    order: u128,
    taker: &Address,
    maker: &Address,
    sold: i128,
    bought: i128,
    left: i128,
) {
    TradeEvent {
        selling: selling.clone(),
        buying: buying.clone(),
        order,
        taker: taker.clone(),
        maker: maker.clone(),
        sold,
        bought,
        left,
    }
    .publish(e);
}

pub(crate) fn emit_swap(
    e: &Env,
    selling: Address,
    buying: Address,
    trader: Address,
    sold: i128,
    bought: i128,
) {
    SwapEvent {
        selling,
        buying,
        trader,
        sold,
        bought,
    }
    .publish(e);
}

pub(crate) fn emit_new(e: &Env, order: &Order) {
    OrderCreatedEvent {
        selling: order.selling.clone(),
        buying: order.buying.clone(),
        id: order.id,
        owner: order.owner.clone(),
        price: order.price,
        amount: order.amount,
        expires: order.expires,
    }
    .publish(e);
}

pub(crate) fn emit_mod(e: &Env, id: u128, price: i128, amount: i128, expires: u64) {
    OrderUpdatedEvent {
        id,
        price,
        amount,
        expires,
    }
    .publish(e);
}
