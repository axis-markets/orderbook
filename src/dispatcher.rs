use crate::errors::OrderbookError;
use crate::events::emit_trade;
use crate::order::Order;
use crate::orderbook::apply_order_trade;
use crate::trade::Trade;
use crate::utils::shorten;
use soroban_sdk::{log, token, Address, Env, Map, Vec};

pub(crate) struct Dispatcher {
    transfers: Map<Address, Map<(Address, Address), i128>>,
    changes: Vec<(Order, i128)>,
    trades: Vec<Trade>,
}

impl Dispatcher {
    pub fn new(e: &Env) -> Dispatcher {
        Dispatcher {
            transfers: Map::new(e),
            changes: Vec::new(e),
            trades: Vec::new(e),
        }
    }

    pub fn add_transfer(&mut self, from: &Address, to: &Address, asset: &Address, amount: i128) {
        let e = self.transfers.env();
        if amount < 0 {
            e.panic_with_error(OrderbookError::Overflow);
        }
        //a self-transfer never changes balances
        if from == to {
            return;
        }
        log!(
            &e,
            "scheduled transfer",
            shorten(from),
            shorten(to),
            shorten(asset),
            amount
        );
        let mut asset_container = self
            .transfers
            .get(asset.clone())
            .unwrap_or_else(|| Map::new(e));
        let key = (from.clone(), to.clone());
        let current = asset_container.get(key.clone()).unwrap_or_default();
        let new_value = current + amount;
        if new_value < 0 {
            e.panic_with_error(OrderbookError::Overflow);
        }
        asset_container.set(key, new_value);
        self.transfers.set(asset.clone(), asset_container);
    }

    pub fn add_order_changes(&mut self, order: Order, change: i128) {
        self.changes.push_back((order, change));
    }

    pub fn add_trade(&mut self, trade: Trade) {
        self.trades.push_back(trade);
    }

    pub fn settle(&self) {
        let e = self.transfers.env();
        //transfer funds
        //TODO: use allowances and transfer funds directly
        for (asset, asset_container) in self.transfers.iter() {
            let client = token::Client::new(&e, &asset);
            for ((from, to), amount) in asset_container.iter() {
                if amount > 0 {
                    log!(
                        &e,
                        "transfer",
                        shorten(&from),
                        shorten(&to),
                        shorten(&asset),
                        amount
                    );
                    client.transfer(&from, &to, &amount);
                }
            }
        }
        //emit trades
        for trade in self.trades.iter() {
            emit_trade(&e, trade.selling.clone(), trade.buying.clone(), trade);
        }
        //apply order changes
        for (mut order, change) in self.changes.iter() {
            apply_order_trade(&e, &mut order, change);
        }
        log!(&e, "settled", self.transfers.len());
    }

    // Transfer tokens
    pub fn transfer(e: &Env, from: &Address, to: &Address, asset: &Address, amount: i128) {
        let client = token::Client::new(&e, &asset);
        log!(
            &e,
            "transfer",
            shorten(from),
            shorten(to),
            shorten(asset),
            amount
        );
        client.transfer(&from, &to.clone(), &amount);
    }
}
