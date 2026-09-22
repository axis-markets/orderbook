//! Settlement of matched fills, aggregated per maker and paid through token allowances.
use crate::events;
use crate::math::{is_dust, mul_div_ceil, PRECISION};
use crate::order::{self, Order};
use soroban_sdk::{contracttype, token, Address, Env, Map, Vec};

/// A planned fill of a maker order
#[contracttype]
#[derive(Clone)]
pub(crate) struct Fill {
    pub order: Order,
    /// Amount of the maker's asset the taker receives
    pub bought: i128,
    /// Amount of the taker's asset the maker receives
    pub sold: i128,
}

pub(crate) struct Dispatcher {
    e: Env,
    /// Account paying for the fills
    taker: Address,
    /// Asset the taker pays with
    pay_asset: Address,
    /// Asset the taker acquires
    get_asset: Address,
    /// Account receiving the acquired asset
    receiver: Address,
    /// Whether to `transfer` out of the contract's own balance (intermediate swap hops)
    intermediate: bool,
    /// Strict: every fill settles exactly as planned or the call fails, otherwise makers are trimmed to their backing or skipped, unbacked orders are removed
    strict: bool,
    /// Pending fills grouped by the maker address
    makers: Map<Address, Vec<Fill>>,
}

impl Dispatcher {
    pub fn new(
        e: &Env,
        taker: Address,
        pay_asset: Address,
        get_asset: Address,
        receiver: Address,
        intermediate: bool,
        strict: bool,
    ) -> Dispatcher {
        Dispatcher {
            e: e.clone(),
            taker,
            pay_asset,
            get_asset,
            receiver,
            intermediate,
            strict,
            makers: Map::new(e),
        }
    }

    /// Add planned settlement to the list
    pub fn add_fill(&mut self, order: Order, bought: i128, sold: i128) {
        //TODO: the backing should be checked when the order is added to the dispatcher (available balance and other props should be cached) to avoid the situation with the partial fill in the end while some valid orders have been skipped
        let maker = order.owner.clone();
        let mut fills = self
            .makers
            .get(maker.clone())
            .unwrap_or_else(|| Vec::new(&self.e));
        fills.push_back(Fill {
            order,
            bought,
            sold,
        });
        self.makers.set(maker, fills);
    }

    /// Totals the fills would settle at if every maker were fully backed
    pub fn planned(&self) -> (i128, i128) {
        let mut sold = 0;
        let mut bought = 0;
        for (_, fills) in self.makers.iter() {
            for fill in fills.iter() {
                sold += fill.sold;
                bought += fill.bought;
            }
        }
        (sold, bought)
    }

    /// Settle every fill maker by maker. Maker's assets go to the receiver, the taker's
    /// asset to the maker.
    /// Returns the amounts the taker actually sold and bought
    pub fn settle(self) -> (i128, i128) {
        let e = &self.e;
        let axis = e.current_contract_address();
        let get_token = token::Client::new(e, &self.get_asset);
        let pay_token = token::Client::new(e, &self.pay_asset);
        //a receiver-side problem is reported before any transfer
        if !self.strict && !self.makers.is_empty() {
            order::require_can_receive(e, &self.get_asset, &self.receiver);
        }
        let mut total_sold = 0;
        let mut total_bought = 0;
        for (maker, fills) in self.makers.iter() {
            //what the maker can deliver: balance and allowance shared by all their orders
            let mut budget = if self.strict {
                i128::MAX
            } else {
                order::backing(e, &self.get_asset, &maker)
            };
            let mut kept: Vec<Fill> = Vec::new(e);
            let mut maker_sends = 0;
            let mut maker_gets = 0;
            for mut fill in fills.iter() {
                let take = fill.bought.min(budget);
                if take <= 0 {
                    continue;
                }
                if take < fill.bought {
                    //trimmed to the backing left
                    fill.bought = take;
                    fill.sold = mul_div_ceil(e, take, fill.order.price, PRECISION);
                }
                budget -= take;
                maker_sends += take;
                maker_gets += fill.sold;
                kept.push_back(fill);
            }
            if maker_sends == 0 {
                //nothing backs the maker's orders: they cannot be filled by anyone
                for fill in fills.iter() {
                    order::remove_with_mod(e, &fill.order);
                }
                continue;
            }
            //maker part
            if self.strict {
                get_token.transfer_from(&axis, &maker, &self.receiver, &maker_sends);
            } else if get_token
                .try_transfer_from(&axis, &maker, &self.receiver, &maker_sends)
                .is_err()
            {
                // the balance is locked elsewhere (classic liabilities, reserve) or the trustline is deauthorized
                // skip the maker, their orders stay untouched
                continue;
            }
            //taker part
            match self.intermediate {
                false => pay_token.transfer_from(&axis, &self.taker, &maker, &maker_gets),
                true => pay_token.transfer(&axis, &maker, &maker_gets),
            }
            //record the fills
            for fill in kept.iter() {
                let mut left = fill.order.amount - fill.bought;
                if !self.strict {
                    //the backing left after this batch caps every order of the maker
                    left = left.min(budget);
                }
                if is_dust(e, left, fill.order.price) {
                    left = 0;
                }
                let mut updated = fill.order.clone();
                order::apply_change(e, &mut updated, left);
                events::emit_trade(
                    e,
                    &self.pay_asset,
                    &self.get_asset,
                    updated.id,
                    &self.taker,
                    &maker,
                    fill.sold,
                    fill.bought,
                    left,
                );
                total_sold += fill.sold;
                total_bought += fill.bought;
            }
        }
        (total_sold, total_bought)
    }
}
