#![no_std]
mod admin;
mod dispatcher;
mod errors;
mod events;
mod market;
mod math;
mod order;
mod orderbook;
mod pricing;
mod reflector_beam;
mod tests;
mod trade;
mod ttl;

use crate::dispatcher::Dispatcher;
use crate::errors::OrderbookError;
use crate::market::{Config, Market};
use crate::math::{check_price, invert_price_ceil, is_dust, mul_div_ceil, PRECISION};
use crate::trade::{Approval, OrderUpdate, TradeStep};
use crate::ttl::bump_contract;
use order::{Order, OrderKind, TradeDirection};
use soroban_sdk::{contract, contractimpl, Address, Env, Map, Vec};

#[contract]
pub struct Axis;

#[contractimpl]
impl Axis {
    /// Create new CLOB contract with a given minimum trade size and oracle.
    ///
    /// # Arguments
    ///
    /// * `safety_admin` - Account allowed to freeze the contract and adjust the oracle
    /// * `oracle` - Price oracle contract address
    /// * `min_trade_size` - Minimum trade value in USD with 7 decimals, zero disables the limit
    ///
    /// # Panics
    ///
    /// If the minimum trade size is negative (`InvalidAmount`)
    /// If the oracle is invalid (`InvalidOracleConfig`)
    pub fn __constructor(e: Env, safety_admin: Address, oracle: Address, min_trade_size: i128) {
        market::init(&e, safety_admin, oracle, min_trade_size);
    }

    /// Fetch contract configuration parameters.
    ///
    /// # Returns
    ///
    /// Safety admin address, price oracle address, market listing fee, and minimum trade size
    pub fn config(e: Env) -> Config {
        market::config(&e)
    }

    /// Check whether the contract is frozen.
    ///
    /// # Returns
    ///
    /// `true` if contract is frozen (trading and order management operations blocked)
    pub fn frozen(e: Env) -> bool {
        admin::is_frozen(&e)
    }

    /// Fetch market for the asset pair.
    ///
    /// # Arguments
    ///
    /// * `selling` - First market asset
    /// * `buying` - Second market asset
    ///
    /// # Returns
    ///
    /// Market record if the market exists
    pub fn market(e: Env, selling: Address, buying: Address) -> Option<Market> {
        market::load_market(&e, &selling, &buying)
    }

    /// Fetch existing order.
    ///
    /// # Arguments
    ///
    /// * `id` - ID of the order to fetch
    ///
    /// # Returns
    ///
    /// Order fetched from the storage, `None` if it does not exist or has expired
    pub fn order(e: Env, id: u128) -> Option<Order> {
        order::load_live_order(&e, id)
    }

    /// Trade with DEX and create a limit order if the quote was not executed in full.
    ///
    ///  # Arguments
    /// * `direction` - Trade direction: `Sell` or `Buy`
    /// * `kind` - Order type (`Limit`, `Fill`, `FillOrKill`)
    /// * `trader` - Trader address
    /// * `amount` - Amount of `selling` tokens to send for `Sell` orders or target amount of `buying` tokens to acquire for `Buy` orders
    /// * `selling` - Token address sent by trader
    /// * `buying` - Token address received by trader
    /// * `price` - Price limit: minimum `buying` per 1 `selling` for `Sell` orders or maximum `selling` per 1 `buying` for `Buy` orders
    /// * `orders` - Optional list of order IDs to match before creating the order on-chain
    /// * `nonce` - Client-chosen nonce for the id of the order created for the remainder
    /// * `expires` - Expiration timestamp of the order created for the remainder (0 = no
    ///   expiration), checked for `Limit` trades only
    /// * `approve` - Optional allowance granted to the contract before trading, normally on `selling`
    ///
    /// # Returns
    ///
    /// * Amount of sold tokens
    /// * Amount of bought tokens
    /// * ID of the newly created order, if any
    ///
    /// # Panics
    ///
    /// If the contract is frozen
    /// If `amount` is invalid, `price` is out of range, or `selling` equals `buying`
    /// If any of the orders provided do not match selling/buying asset
    /// If the trader cannot pay for the fills, or cannot receive `buying`
    /// If a `FillOrKill` trade cannot be executed in full (`NotFilled`)
    /// If the remainder is not backed by the trader's balance and allowance
    /// If an order with the same id is already on the book (`OrderExists`)
    /// If a `Limit` trade `expires` is set in the past (`InvalidExpiration`)
    /// If a `Limit` trade's market does not exist
    /// If a `Limit` trade sells less than the minimum order value (`OrderSizeTooSmall`)
    /// If neither of the order assets is quoted by the oracle (`AssetsNotVerifiedByOracle`)
    /// If no usable price is cached for the asset it is valued on (`AssetPriceOracleFetchFailed`)
    #[allow(clippy::too_many_arguments)]
    pub fn trade(
        e: Env,
        direction: TradeDirection,
        kind: OrderKind,
        trader: Address,
        amount: i128,
        selling: Address,
        buying: Address,
        price: i128,
        orders: Vec<u128>,
        nonce: u64,
        expires: u64,
        approve: Option<Approval>,
    ) -> (i128, i128, Option<u128>) {
        //need permission from the trader
        trader.require_auth();
        //keep contract alive
        bump_contract(&e);
        //no trading while the contract is frozen
        admin::require_not_frozen(&e);
        //validate trade parameters
        if amount <= 0 {
            e.panic_with_error(OrderbookError::InvalidAmount);
        }
        check_price(&e, price);
        if selling == buying {
            e.panic_with_error(OrderbookError::InvalidMatch);
        }
        //grant the allowance the trade is paid from
        if let Some(approval) = &approve {
            trade::approve(&e, &trader, approval);
        }
        //verify the expiration, the market and the order size
        if kind == OrderKind::Limit {
            order::check_expires(&e, expires);
            let market = market::load_listed_market(&e, &selling, &buying);
            pricing::enforce_min_order_value(
                &e, &market, &direction, amount, price, &selling, &buying,
            );
        }
        let mut dispatcher = Dispatcher::new(
            &e,
            trader.clone(),
            selling.clone(),
            buying.clone(),
            trader.clone(),
            false,
            false,
        );
        //match the supplied orders
        if !orders.is_empty() {
            let max_exec_price = orderbook::get_price_threshold(&e, &direction, price);
            orderbook::match_orders(
                &e,
                &direction,
                amount,
                &selling,
                &buying,
                max_exec_price,
                &orders,
                None,
                &mut dispatcher,
            );
        }
        let filled_of = |sold: i128, bought: i128| match direction {
            TradeDirection::Sell => sold,
            TradeDirection::Buy => bought,
        };
        //FillOrKill does not allow partial execution: fail before moving anything
        if kind == OrderKind::FillOrKill {
            let (sold, bought) = dispatcher.planned();
            if filled_of(sold, bought) < amount {
                e.panic_with_error(OrderbookError::NotFilled);
            }
        }
        //settle all fills
        let (sold, bought) = dispatcher.settle();
        let filled = filled_of(sold, bought);
        //a maker trimmed or skipped at settlement leaves a FillOrKill short
        if kind == OrderKind::FillOrKill && filled < amount {
            e.panic_with_error(OrderbookError::NotFilled);
        }
        //return if executed in full or partial execution requested
        if filled >= amount || kind != OrderKind::Limit {
            return (sold, bought, None);
        }
        //not executed in full, need to create a limit order for the remainder
        //orders are always stored in sell-equivalent form
        let (order_amount, order_price) = match direction {
            TradeDirection::Sell => (amount - sold, price),
            TradeDirection::Buy => (
                mul_div_ceil(&e, amount - bought, price, PRECISION),
                invert_price_ceil(&e, price),
            ),
        };
        //a remainder worth less than one unit of the counter asset cannot be filled
        if is_dust(&e, order_amount, order_price) {
            if filled == 0 {
                e.panic_with_error(OrderbookError::OrderSizeTooSmall);
            }
            return (sold, bought, None);
        }
        //the id must be free, the remainder backed by the trader's balance and allowance
        let id = order::order_id(&e, &trader, nonce);
        order::require_free(&e, id);
        order::require_backed(&e, &trader, &selling, order_amount);
        order::require_can_receive(&e, &buying, &trader);
        order::store_order(
            &e,
            id,
            trader,
            order_amount,
            selling,
            buying,
            order_price,
            expires,
        );
        (sold, bought, Some(id))
    }

    /// Update the amount, price, or expiration of several orders in place, or remove them.
    /// Orders that no longer exist are skipped; expired orders revived with the new expiration.
    /// A zero amount removes the order, expired ones included.
    ///
    /// # Arguments
    ///
    /// * `trader` - Orders owner
    /// * `updates` - New amount, price, and expiration per order
    /// * `approvals` - Allowances granted to the contract before the backing check; an asset
    ///   without an approval keeps its current allowance
    ///
    /// # Returns
    ///
    /// IDs of the orders updated or removed
    ///
    /// # Panics
    ///
    /// If the contract is frozen
    /// If `trader` does not own an updated order (`NotAuthorized`)
    /// If an `amount` is negative or `price` is out of range
    /// If an `expires` is set in the past (`InvalidExpiration`)
    /// If the new amounts are not backed by the trader's balance and allowance
    /// If an order is below the minimum value
    /// If no usable price is cached for the asset it is valued on (`AssetPriceOracleFetchFailed`)
    pub fn update(
        e: Env,
        trader: Address,
        updates: Vec<OrderUpdate>,
        approvals: Vec<Approval>,
    ) -> Vec<u128> {
        trader.require_auth();
        bump_contract(&e);
        admin::require_not_frozen(&e);
        //grant the allowances the updated orders are backed by
        for approval in approvals.iter() {
            trade::approve(&e, &trader, &approval);
        }
        let mut updated: Vec<u128> = Vec::new(&e);
        //amount to back per selling asset
        let mut required: Map<Address, i128> = Map::new(&e);
        for update in updates.iter() {
            //an expired order stays revivable, `check_expires` below keeps the new expiry valid
            let mut existing = match order::load_order(&e, update.id) {
                Some(o) => o,
                None => continue,
            };
            if existing.owner != trader {
                e.panic_with_error(OrderbookError::NotAuthorized);
            }
            //a zero amount removes the order: no price, expiration, market or backing checks
            if update.amount == 0 {
                order::remove_with_mod(&e, &existing);
                updated.push_back(existing.id);
                continue;
            }
            if update.amount < 0 {
                e.panic_with_error(OrderbookError::InvalidAmount);
            }
            check_price(&e, update.price);
            order::check_expires(&e, update.expires);
            if is_dust(&e, update.amount, update.price) {
                e.panic_with_error(OrderbookError::OrderSizeTooSmall);
            }
            let market = match market::load_market(&e, &existing.selling, &existing.buying) {
                Some(market) => market,
                None => e.panic_with_error(OrderbookError::OrderNotFound),
            };
            //stored orders are sell-equivalent
            pricing::enforce_min_order_value(
                &e,
                &market,
                &TradeDirection::Sell,
                update.amount,
                update.price,
                &existing.selling,
                &existing.buying,
            );
            order::require_can_receive(&e, &existing.buying, &trader);
            let total = required.get(existing.selling.clone()).unwrap_or(0) + update.amount;
            required.set(existing.selling.clone(), total);
            order::modify(
                &e,
                &mut existing,
                update.amount,
                update.price,
                update.expires,
            );
            updated.push_back(existing.id);
        }
        for (asset, amount) in required.iter() {
            order::require_backed(&e, &trader, &asset, amount);
        }
        updated
    }

    /// Fill an existing order against matching orders from the orderbook.
    /// Profits from inefficiencies go to the trader.
    ///
    ///  # Arguments
    /// * `trader` - Trader address
    /// * `taker_order_id` - ID of the order that serves as a taker
    /// * `orders` - List of order IDs to match against
    ///
    /// # Returns
    ///
    /// * Amount the taker order sold
    /// * Amount the makers delivered
    /// * Surplus paid to `trader`
    ///
    /// # Panics
    ///
    /// If the contract is frozen
    /// If the taker order does not exist or has expired (`OrderNotFound`)
    /// If any of the orders provided do not match selling/buying asset
    /// If the taker order's owner or `trader` cannot receive the bought asset
    pub fn crossfill(
        e: Env,
        trader: Address,
        taker_order_id: u128,
        orders: Vec<u128>,
    ) -> (i128, i128, i128) {
        trader.require_auth();
        bump_contract(&e);
        admin::require_not_frozen(&e);
        orderbook::cross_orders(&e, &trader, taker_order_id, &orders)
    }

    /// Swap tokens across several markets.
    /// The contract holds the intermediate hop proceeds only within the call.
    ///
    /// # Arguments
    /// * `direction` - Trade direction: `Sell` or `Buy`
    /// * `trader` - Trader address
    /// * `selling` - Token address sent by the trader
    /// * `selling_amount` - Amount of selling tokens to send (`Sell`) or the maximum to spend (`Buy`)
    /// * `buying_amount` - Minimum amount of buying tokens to receive (`Sell`) or the exact amount (`Buy`)
    /// * `path` - Ordered list of the trade route steps
    /// * `approve` - Optional allowance granted to the contract before trading, normally on `selling`
    ///
    /// # Returns
    ///
    /// * Amount of sold tokens
    /// * Amount of bought tokens
    ///
    /// # Panics
    ///
    /// If the contract is frozen
    /// If `path` is empty or an amount is not positive
    /// If the route cannot satisfy the selling/buying amount (`NotFilled`)
    /// If any of the orders provided do not match trade step selling/buying asset
    /// If the trader cannot pay for the fills, or cannot receive `buying`
    #[allow(clippy::too_many_arguments)]
    pub fn swap(
        e: Env,
        direction: TradeDirection,
        trader: Address,
        selling: Address,
        selling_amount: i128,
        buying_amount: i128,
        path: Vec<TradeStep>,
        approve: Option<Approval>,
    ) -> (i128, i128) {
        trader.require_auth();
        bump_contract(&e);
        admin::require_not_frozen(&e);

        let hops = path.len();
        if hops == 0 || selling_amount <= 0 || buying_amount <= 0 {
            e.panic_with_error(OrderbookError::InvalidMatch);
        }
        if let Some(approval) = &approve {
            trade::approve(&e, &trader, approval);
        }

        let axis = e.current_contract_address();
        let sell_asset = |i: u32| {
            if i == 0 {
                selling.clone()
            } else {
                path.get(i - 1).unwrap().asset
            }
        };
        //accept every supplied order regardless of price; slippage is checked on the aggregate
        let no_price_limit = i128::MAX;

        //plan every hop read-only, (amount to match, sold, bought) per hop
        let mut plan: Vec<(i128, i128, i128)> = Vec::new(&e);
        let plan_hop = |i: u32, amount: i128| {
            let step = path.get(i).unwrap();
            let mut dispatcher = Dispatcher::new(
                &e,
                axis.clone(),
                sell_asset(i),
                step.asset.clone(),
                axis.clone(),
                true,
                true,
            );
            orderbook::match_orders(
                &e,
                &direction,
                amount,
                &sell_asset(i),
                &step.asset,
                no_price_limit,
                &step.orders,
                None,
                &mut dispatcher,
            );
            dispatcher.planned()
        };
        match direction {
            //fixed input: each hop sells the entire output of the previous one, front to back
            TradeDirection::Sell => {
                let mut input = selling_amount;
                for i in 0..hops {
                    let (sold, bought) = plan_hop(i, input);
                    if sold != input || bought == 0 {
                        e.panic_with_error(OrderbookError::NotFilled);
                    }
                    plan.push_back((input, sold, bought));
                    input = bought;
                }
                if input < buying_amount {
                    e.panic_with_error(OrderbookError::NotFilled);
                }
            }
            //fixed output: back to front, calculating the input each hop must deliver
            TradeDirection::Buy => {
                let mut output = buying_amount;
                let mut i = hops;
                while i > 0 {
                    i -= 1;
                    let (sold, bought) = plan_hop(i, output);
                    if bought != output || sold == 0 {
                        e.panic_with_error(OrderbookError::NotFilled);
                    }
                    plan.push_front((output, sold, bought));
                    output = sold;
                }
                if output > selling_amount {
                    e.panic_with_error(OrderbookError::NotFilled);
                }
            }
        }

        //execute front to back; the trader pays the first hop, the contract pays the
        //later hops out of what the previous hop delivered and the last hop pays the trader
        let mut total_sold = 0;
        let mut total_bought = 0;
        for i in 0..hops {
            let step = path.get(i).unwrap();
            let (amount, planned_sold, planned_bought) = plan.get(i).unwrap();
            let first = i == 0;
            let last = i == hops - 1;
            let mut dispatcher = Dispatcher::new(
                &e,
                if first { trader.clone() } else { axis.clone() },
                sell_asset(i),
                step.asset.clone(),
                if last { trader.clone() } else { axis.clone() },
                !first,
                true,
            );
            orderbook::match_orders(
                &e,
                &direction,
                amount,
                &sell_asset(i),
                &step.asset,
                no_price_limit,
                &step.orders,
                None,
                &mut dispatcher,
            );
            let (sold, bought) = dispatcher.settle();
            if sold != planned_sold || bought != planned_bought {
                e.panic_with_error(OrderbookError::NotFilled);
            }
            if first {
                total_sold = sold;
            }
            if last {
                total_bought = bought;
            }
        }
        let dest_asset = path.get(hops - 1).unwrap().asset;
        events::emit_swap(&e, selling, dest_asset, trader, total_sold, total_bought);
        (total_sold, total_bought)
    }

    /// Re-check both market assets against the price oracle and cache oracle prices.
    /// A cached price is valid for up to 72 hours. A market without quoted assets stops accepting
    /// new limit orders; its outstanding orders stay cancellable and fillable.
    ///
    /// # Arguments
    ///
    /// * `selling` - First market asset
    /// * `buying` - Second market asset
    ///
    /// # Returns
    ///
    /// Updated market record, `None` if the market does not exist
    ///
    /// # Panics
    ///
    /// If the contract is frozen
    pub fn requote(e: Env, selling: Address, buying: Address) -> Option<Market> {
        bump_contract(&e);
        admin::require_not_frozen(&e);
        let market = market::refresh(&e, &selling, &buying)?;
        pricing::fetch_prices(&e, &market);
        Some(market)
    }

    /// Extend oracle price feeds access for a market, creating the market if it does not exist yet.
    ///
    /// # Arguments
    ///
    /// * `sponsor` - Address paying for the oracle feeds
    /// * `selling` - First market asset
    /// * `buying` - Second market asset
    /// * `amount` - Amount of fee tokens to burn
    ///
    /// # Returns
    ///
    /// New access expiration UNIX timestamps (in seconds) per oracle-listed asset
    ///
    /// # Panics
    ///
    /// If `amount` is invalid
    /// If the market does not exist and the amount is below the market listing fee (`InvalidAmount`)
    /// If neither market asset is quoted by the oracle (`AssetsNotVerifiedByOracle`)
    /// If the contract is frozen
    pub fn subsidize(
        e: Env,
        sponsor: Address,
        selling: Address,
        buying: Address,
        amount: i128,
    ) -> Vec<u64> {
        sponsor.require_auth();
        bump_contract(&e);
        admin::require_not_frozen(&e);
        if amount <= 0 {
            e.panic_with_error(OrderbookError::InvalidAmount);
        }
        market::fund(&e, &sponsor, &selling, &buying, amount)
    }

    /// Toggle freezing the contract. A frozen DEX blocks every trading and order management call.
    ///
    /// # Arguments
    ///
    /// * `blocked` - `true` blocks trading, `false` resumes normal operation
    ///
    /// # Panics
    ///
    /// If the call is not authorized by the safety admin
    pub fn freeze(e: Env, blocked: bool) {
        //only the safety admin operates the emergency switch
        admin::require_safety_admin(&e);
        bump_contract(&e);
        admin::set_frozen(&e, blocked);
    }

    /// Hand the safety admin role over to another account.
    ///
    /// # Arguments
    ///
    /// * `new_safety_admin` - Account that takes over the safety admin role
    ///
    /// # Panics
    ///
    /// If the call is not authorized by the current safety admin
    pub fn delegate(e: Env, new_safety_admin: Address) {
        admin::require_safety_admin(&e);
        bump_contract(&e);
        admin::set_safety_admin(&e, &new_safety_admin);
    }

    /// Point the contract at another price oracle.
    ///
    /// # Arguments
    ///
    /// * `oracle` - Reflector Beam price oracle contract address
    ///
    /// # Panics
    ///
    /// If the call is not authorized by the safety admin
    /// If the oracle quotes prices with too many decimals, or charges no daily fee
    /// (`InvalidOracleConfig`)
    pub fn set_oracle(e: Env, oracle: Address) {
        admin::require_safety_admin(&e);
        bump_contract(&e);
        admin::set_oracle(&e, &oracle);
    }

    /// Set the protocol minimum trade size.
    ///
    /// # Arguments
    ///
    /// * `minimum` - Minimum trade value in USD with 7 decimals (1 USD = 10_000_000)
    ///
    /// # Panics
    ///
    /// If the call is not authorized by the safety admin
    /// If `minimum` is negative
    pub fn set_floor(e: Env, minimum: i128) {
        admin::require_safety_admin(&e);
        bump_contract(&e);
        admin::set_min_trade_size(&e, minimum);
    }

    /// Extend the contract instance and code lifetime.
    pub fn keepalive(e: Env) {
        bump_contract(&e);
    }
}
