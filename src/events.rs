use crate::order::Order;
use crate::trade::Trade;
use soroban_sdk::{contractevent, Address, Env, Symbol};

#[contractevent(topics = ["AXIS", "trade"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeEvent {
    /// Sold asset address
    #[topic]
    pub selling: Address,
    /// Bought asset address
    #[topic]
    //TODO: consider adding taker and owner addresses
    pub buying: Address,
    /// Trade details
    pub trade: Trade,
}

#[contractevent(topics = ["AXIS", "order"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderEvent {
    /// Order change type: "created"|"updated"|"removed"
    #[topic]
    pub action: Symbol,
    /// Selling asset address
    #[topic]
    pub selling: Address,
    /// Buying asset address
    #[topic]
    //TODO: consider adding taker and owner addresses
    pub buying: Address,
    /// Order details
    pub order: Order,
}

pub(crate) fn emit_trade(e: &Env, selling: Address, buying: Address, trade: Trade) {
    TradeEvent {
        selling,
        buying,
        trade,
    }
    .publish(e);
}

pub(crate) fn emit_order_event(e: &Env, action: Symbol, order: Order) {
    OrderEvent {
        action,
        selling: order.selling.clone(),
        buying: order.buying.clone(),
        order,
    }
    .publish(e); //TODO: consider adding owner to topics
}
