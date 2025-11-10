use crate::order::Order;
use crate::trade::Trade;
use soroban_sdk::{symbol_short, Env, Symbol};

const SELF: Symbol = symbol_short!("orderbook");

//TODO: use #[contractevent] macro: https://docs.rs/soroban-sdk/23.0.2/soroban_sdk/attr.contractevent.html
pub fn emit_trade(e: &Env, trade: Trade) {
    e.events().publish(
        (
            //TODO: consider adding taker and owner addresses
            SELF,
            symbol_short!("trade"),
            trade.selling.clone(),
            trade.buying.clone(),
        ),
        trade,
    );
}

pub fn emit_order_event(e: &Env, event: Symbol, order: Order) {
    e.events().publish(
        //TODO: consider adding kind and owner
        (SELF, event, order.selling.clone(), order.buying.clone()),
        order,
    );
}
