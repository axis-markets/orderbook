use crate::order::Order;
use crate::trade::{next_trade_id, Swap, Trade};
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

#[contractevent(topics = ["AXIS", "swap"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapEvent {
    /// Asset sold by the trader
    #[topic]
    pub selling: Address,
    /// Asset received by the trader
    #[topic]
    pub buying: Address,
    /// Swap details
    pub swap: Swap,
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

pub(crate) fn emit_trade(e: &Env, selling: Address, buying: Address, mut trade: Trade) {
    trade.id = next_trade_id(&e);
    //log!(e, "evt:trade", trade.clone());
    TradeEvent {
        selling,
        buying,
        trade,
    }
    .publish(e);
}

pub(crate) fn emit_swap(
    e: &Env,
    trader: Address,
    selling: Address,
    buying: Address,
    sold: i128,
    bought: i128,
) {
    let swap = Swap {
        id: next_trade_id(&e),
        trader,
        selling: selling.clone(),
        buying: buying.clone(),
        sold,
        bought,
    };
    //log!(e, "evt:swap", sold, bought);
    SwapEvent {
        selling,
        buying,
        swap,
    }
    .publish(e);
}

pub(crate) fn emit_order_event(e: &Env, action: Symbol, order: Order) {
    //log!(e, "evt:order", action, order.clone());
    OrderEvent {
        action,
        selling: order.selling.clone(),
        buying: order.buying.clone(),
        order,
    }
    .publish(e);
}
