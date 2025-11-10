use crate::events;
use soroban_sdk::{contracttype, symbol_short, Address, Env};

const LAST_ORDER_ID: &str = "lo";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderType {
    Limit,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TradeDirection {
    Buy,
    Sell,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Order {
    pub id: u64,
    //order type
    pub kind: OrderType,
    //selling token address
    pub selling: Address,
    //buying token address
    pub buying: Address,
    //selling amount left
    pub amount: i128,
    //initial selling amount
    pub quote: i128,
    //maker address
    pub owner: Address,
    //order price
    pub price: i128,
    //expiration timestamp
    pub expires: u64,
}

pub fn get_last_order_id(e: &Env) -> u64 {
    e.storage().instance().get(&LAST_ORDER_ID).unwrap_or(0)
}

fn next_order_id(e: &Env) -> u64 {
    let last = get_last_order_id(&e) + 1;
    e.storage().instance().set(&LAST_ORDER_ID, &last);
    last
}

pub fn create_order(
    e: &Env,
    kind: OrderType,
    trader: Address,
    amount: i128,
    selling: Address,
    buying: Address,
    price: i128,
    ttl: u64,
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
        expires: e.ledger().timestamp() + ttl,
    };
    e.storage().persistent().set(&id, &new_order);
    events::emit_order_event(e, symbol_short!("created"), new_order);
    id
}

pub fn remove_order(e: &Env, order: &Order) {
    e.storage().persistent().remove(&order.id);
    events::emit_order_event(e, symbol_short!("removed"), order.clone())
}

pub fn update_order(e: &Env, order: &Order) {
    e.storage().persistent().set(&order.id, order);
    events::emit_order_event(e, symbol_short!("updated"), order.clone())
}

pub fn load_order(e: &Env, id: &u64) -> Option<Order> {
    e.storage().persistent().get(&id)
}
