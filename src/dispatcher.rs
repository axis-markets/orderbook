//! Settlement of matched fills, aggregated per maker and paid through token allowances.
use crate::errors::OrderbookError;
use crate::events;
use crate::math::is_dust;
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

/// What a maker can settle, read on the maker's first fill
#[contracttype]
#[derive(Clone)]
struct MakerCapacity {
    /// Whether the maker can receive the asset the taker pays with
    receivable: bool,
    /// Backing left for further fills: balance and allowance shared by all the maker's orders
    budget: i128,
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
    /// Strict: every admitted fill settles exactly as planned or the call fails, otherwise a
    /// maker whose transfer fails is skipped
    strict: bool,
    /// Pending fills grouped by the maker address
    makers: Map<Address, Vec<Fill>>,
    /// Settlement capacity per maker
    capacity: Map<Address, MakerCapacity>,
    /// Listed orders their makers could not settle, reported with `skip` events
    skipped: Vec<u128>,
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
            capacity: Map::new(e),
            skipped: Vec::new(e),
        }
    }

    /// Queue a fill if the maker can settle it: their backing left covers `bought` and they can
    /// receive the taker's asset. Otherwise the order is recorded as skipped and left unchanged.
    /// Returns whether the fill was queued
    pub fn add_fill(&mut self, order: Order, bought: i128, sold: i128) -> bool {
        let e = &self.e;
        let maker = order.owner.clone();
        let mut capacity = match self.capacity.get(maker.clone()) {
            Some(capacity) => capacity,
            None => {
                let receivable = order::can_receive(e, &self.pay_asset, &maker);
                //the backing of a maker who cannot be paid does not matter
                let budget = if receivable {
                    order::backing(e, &self.get_asset, &maker)
                } else {
                    0
                };
                MakerCapacity { receivable, budget }
            }
        };
        let admitted = capacity.receivable && capacity.budget >= bought;
        if admitted {
            capacity.budget -= bought;
            let mut fills = self
                .makers
                .get(maker.clone())
                .unwrap_or_else(|| Vec::new(e));
            fills.push_back(Fill {
                order,
                bought,
                sold,
            });
            self.makers.set(maker.clone(), fills);
        } else {
            self.skipped.push_back(order.id);
        }
        self.capacity.set(maker, capacity);
        admitted
    }

    /// Totals of the queued fills
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

    /// Highest order price among the queued fills, 0 when nothing is queued
    pub fn worst_price(&self) -> i128 {
        let mut worst = 0;
        for (_, fills) in self.makers.iter() {
            for fill in fills.iter() {
                worst = worst.max(fill.order.price);
            }
        }
        worst
    }

    /// Settle every fill maker by maker. Maker's assets go to the receiver, the taker's
    /// asset to the maker. Skipped orders are reported with `skip` events. A receiver that refuses
    /// the acquired asset fails the call (`CannotReceive`) rather than skipping the makers.
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
        let mut skipped = self.skipped;
        let mut total_sold = 0;
        let mut total_bought = 0;
        for (maker, fills) in self.makers.iter() {
            let mut maker_sends = 0;
            let mut maker_gets = 0;
            for fill in fills.iter() {
                maker_sends += fill.bought;
                maker_gets += fill.sold;
            }
            //maker part
            if self.strict {
                get_token.transfer_from(&axis, &maker, &self.receiver, &maker_sends);
            } else if get_token
                .try_transfer_from(&axis, &maker, &self.receiver, &maker_sends)
                .is_err()
            {
                //a maker who can move the amount to themselves is ok - the receiver refused it
                if self.receiver != axis
                    && get_token
                        .try_transfer_from(&axis, &maker, &maker, &maker_sends)
                        .is_ok()
                {
                    e.panic_with_error(OrderbookError::CannotReceive);
                }
                // the backing read passed, so the balance is locked elsewhere (classic liabilities,
                // reserve) or the trustline is deauthorized: skip the maker, their orders stay untouched
                for fill in fills.iter() {
                    skipped.push_back(fill.order.id);
                }
                continue;
            }
            //taker part
            match self.intermediate {
                false => pay_token.transfer_from(&axis, &self.taker, &maker, &maker_gets),
                true => pay_token.transfer(&axis, &maker, &maker_gets),
            }
            //record the fills
            for fill in fills.iter() {
                let mut left = fill.order.amount - fill.bought;
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
        for id in skipped.iter() {
            events::emit_skip(e, id);
        }
        (total_sold, total_bought)
    }
}
