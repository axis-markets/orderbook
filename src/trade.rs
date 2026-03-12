use soroban_sdk::{contracttype, Address, Env};

const LAST_TRADE_ID: &str = "lt";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trade {
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
}

pub fn get_last_trade_id(e: &Env) -> u64 {
    e.storage().instance().get(&LAST_TRADE_ID).unwrap_or(0)
}

pub fn next_trade_id(e: &Env) -> u64 {
    let last = get_last_trade_id(&e) + 1;
    e.storage().instance().set(&LAST_TRADE_ID, &last);
    last
}
