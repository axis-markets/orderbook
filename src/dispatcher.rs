use crate::errors::OrderbookError;
use crate::utils::shorten;
use soroban_sdk::{log, token, Address, Env, Map};

pub(crate) struct Dispatcher {
    map: Map<Address, Map<(Address, Address), i128>>,
}

impl Dispatcher {
    pub fn new(e: &Env) -> Dispatcher {
        Dispatcher { map: Map::new(e) }
    }

    pub fn add(&mut self, from: &Address, to: &Address, asset: &Address, amount: i128) {
        let e = self.map.env();
        if amount < 0 {
            e.panic_with_error(OrderbookError::Overflow);
        }
        log!(
            &e,
            "scheduled transfer",
            shorten(from),
            shorten(to),
            shorten(asset),
            amount
        );
        let mut asset_container = self.map.get(asset.clone()).unwrap_or_else(|| Map::new(e));
        let key = (from.clone(), to.clone());
        let current = asset_container.get(key.clone()).unwrap_or_default();
        let new_value = current + amount;
        if new_value < 0 {
            e.panic_with_error(OrderbookError::Overflow);
        }
        asset_container.set(key, new_value);
        self.map.set(asset.clone(), asset_container);
    }

    pub fn settle(&self) {
        let e = self.map.env();
        log!(&e, "settle", self.map.len());
        for (asset, asset_container) in self.map.iter() {
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

        /*let selling_client = token::Client::new(&e, &selling);
        let buying_client = token::Client::new(&e, &buying);
        //TODO: use allowances and transfer funds directly
        //transfer sold tokens from trader (taker) to contract
        selling_client.transfer(&trader, &this, &total_sold);
        for trade in trades.iter() {
            //transfer funds from contract to maker
            //TODO: group by taker/maker pair before settling
            selling_client.transfer(&this, &trade.maker, &trade.sold);
        }
        //transfer bought tokens to trader
        buying_client.transfer(&this, &trader, &total_bought);*/
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
