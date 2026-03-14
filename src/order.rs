use crate::events;
use soroban_sdk::{contracttype, symbol_short, Address, Env};

const LAST_ORDER_ID: &str = "lo";
//const DEFAULT_TTL: u64 = 30 * 24 * 60 * 60; //30 days

/// Trading order type - instructions to contract how to execute the trade
#[contracttype]
#[repr(i16)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderKind {
    /// Execute trade, create a limit order if not executed in full
    Limit = 1,
    /// Execute trade without creating a limit order
    Fill = 2,
    /// Execute trade, cancel if was not executed in full
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
    ///Unique order identifier
    pub id: u64,
    ///Order type
    pub kind: OrderKind,
    ///Selling token address
    pub selling: Address,
    ///Buying token address
    pub buying: Address,
    ///Amount left to sell/buy
    pub amount: i128,
    /// Initial selling/buying amount
    pub quote: i128,
    ///Maker address
    pub owner: Address,
    ///Order price
    pub price: i128,
    ///expiration timestamp
    pub expires: u64,
}

pub(crate) fn get_last_order_id(e: &Env) -> u64 {
    e.storage().instance().get(&LAST_ORDER_ID).unwrap_or(0)
}

fn next_order_id(e: &Env) -> u64 {
    let last = get_last_order_id(&e) + 1;
    e.storage().instance().set(&LAST_ORDER_ID, &last);
    last
}

pub(crate) fn create_order(
    e: &Env,
    kind: OrderKind,
    trader: Address,
    amount: i128,
    selling: Address,
    buying: Address,
    price: i128,
) -> u64 {
    //get next sequential order id
    let id = next_order_id(&e);
    //compose order record
    let new_order = Order {
        id,
        kind,
        owner: trader,
        quote: amount,
        amount,
        selling,
        buying,
        price,
        expires: 0,
    };
    e.storage().persistent().set(&id, &new_order);
    events::emit_order_event(e, symbol_short!("created"), new_order);
    id
}

pub(crate) fn remove_order(e: &Env, order: &Order) {
    e.storage().persistent().remove(&order.id);
    events::emit_order_event(e, symbol_short!("removed"), order.clone())
}

pub(crate) fn update_order(e: &Env, order: &Order) {
    e.storage().persistent().set(&order.id, order);
    events::emit_order_event(e, symbol_short!("updated"), order.clone())
}

pub(crate) fn load_order(e: &Env, id: &u64) -> Option<Order> {
    e.storage().persistent().get(&id)
}
