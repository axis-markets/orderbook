use crate::order::Order;
use crate::trade::Trade;
use soroban_sdk::{contractevent, Address, Env, Symbol};

/*pub struct TradeInfo {
    //unique trade id
    pub id: u64,
    //order id
    pub order: u64,
    //trader account address
    pub taker: Address,
    //seller account address
    pub maker: Address,
    //sold tokens amount
    pub sold: i128,
    //bought tokens amount
    pub bought: i128,
}*/

#[contractevent(topics = ["AXIS", "trade"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeEvent {
    #[topic]
    pub selling: Address,
    #[topic]
    //TODO: consider adding taker and owner addresses
    pub buying: Address,
    pub trade: Trade,
}

#[contractevent(topics = ["AXIS", "order"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderEvent {
    #[topic]
    pub action: Symbol,
    #[topic]
    pub selling: Address,
    #[topic]
    //TODO: consider adding taker and owner addresses
    pub buying: Address,
    pub order: Order,
}

pub fn emit_trade(e: &Env, selling: Address, buying: Address, trade: Trade) {
    TradeEvent {
        selling,
        buying,
        trade,
    }
    .publish(e);
}

pub fn emit_order_event(e: &Env, action: Symbol, order: Order) {
    OrderEvent {
        action,
        selling: order.selling.clone(),
        buying: order.buying.clone(),
        order,
    }
    .publish(e); //TODO: consider adding kind and owner
}
