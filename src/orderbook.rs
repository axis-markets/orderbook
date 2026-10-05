//! Matching engine: fills a taker against a list of maker orders and crosses book orders.
use crate::dispatcher::Dispatcher;
use crate::errors::OrderbookError;
use crate::events;
use crate::math::{invert_price_floor, is_dust, mul_div_ceil, mul_div_floor, try_mul_div_ceil};
use crate::order::{self, load_live_order, load_order, Order, TradeDirection};
use soroban_sdk::{token, Address, Env, Vec};

pub use crate::math::PRECISION;

/// Resolve taker-provided `max_price` into the threshold to compare against maker `order.price`.
pub(crate) fn get_price_threshold(e: &Env, direction: &TradeDirection, max_price: i128) -> i128 {
    match direction {
        TradeDirection::Sell => invert_price_floor(e, max_price),
        TradeDirection::Buy => max_price,
    }
}

/// Match a taker against a list of maker orders, queueing the fills on the dispatcher.
/// Depending on `direction`:
/// - Sell: `amount` is in `selling` units
/// - Buy : `amount` is in `buying` units
///
/// `max_exec_price` is the acceptance threshold (from `get_price_threshold` or `i128::MAX`).
/// With `profit_floor` (a stored taker order's price) a fill is skipped unless it yields at
/// least what that order is owed for the amount it pays.
/// A fill the maker cannot settle (backing short of it, or unable to receive the taker's asset)
/// is skipped and reported by the dispatcher.
/// Stops as soon as the taker is filled; duplicate and unusable ids are skipped
#[allow(clippy::too_many_arguments)]
pub(crate) fn match_orders(
    e: &Env,
    direction: &TradeDirection,
    amount: i128,
    selling: &Address,
    buying: &Address,
    max_exec_price: i128,
    orders: &Vec<u128>,
    profit_floor: Option<i128>,
    dispatcher: &mut Dispatcher,
) {
    let now = e.ledger().timestamp();
    let mut amount_left = amount; //sell-units for Sell, buy-units for Buy
    let mut seen: Vec<u128> = Vec::new(e);
    for maker_order_id in orders.iter() {
        //an order listed twice fills once
        if seen.contains(maker_order_id) {
            continue;
        }
        seen.push_back(maker_order_id);
        let order = match load_matching_order(e, maker_order_id, selling, buying, now) {
            Some(o) => o,
            None => continue,
        };
        //skip orders priced worse than requested
        if order.price > max_exec_price {
            continue;
        }
        //compute per-order trade amounts in taker units
        let (sold, bought) = match direction {
            TradeDirection::Sell => sell_amounts_for(e, &order, amount_left),
            TradeDirection::Buy => buy_amounts_for(e, &order, amount_left),
        };
        //helpers return (0, 0) when this order cannot be executed
        if sold == 0 || bought == 0 {
            continue;
        }
        //rounding must never leave the stored taker order underpaid
        if let Some(taker_price) = profit_floor {
            match try_mul_div_ceil(e, sold, taker_price, PRECISION) {
                Some(owed) if bought >= owed => {}
                //an amount owed beyond i128 cannot be delivered
                _ => continue,
            }
        }
        //the maker must be able to settle the fill, otherwise the order is skipped
        if !dispatcher.add_fill(order, bought, sold) {
            continue;
        }
        //decrement remaining work in the direction-appropriate unit
        amount_left -= match direction {
            TradeDirection::Sell => sold,
            TradeDirection::Buy => bought,
        };
        if amount_left <= 0 {
            break;
        }
    }
}

/// How much the taker can sell to this order at its price: the acquired amount is rounded
/// down, the amount paid for it rounded up, so the maker is never underpaid. The whole order is
/// priced first, so a taker amount worth far more than the order is never converted in full.
/// Returns (sold, bought) or (0, 0) if the trade can't be executed
fn sell_amounts_for(e: &Env, order: &Order, amount_left: i128) -> (i128, i128) {
    //the whole order if the taker can pay for it; a cost beyond i128 exceeds any taker amount
    if let Some(cost) = try_mul_div_ceil(e, order.amount, order.price, PRECISION) {
        if amount_left >= cost {
            return (cost, order.amount);
        }
    }
    //a part of the order: below its amount, so the result fits
    let bought = mul_div_floor(e, amount_left, PRECISION, order.price);
    if bought == 0 {
        return (0, 0);
    }
    //never above amount_left: bought <= amount_left * PRECISION / price
    let sold = mul_div_ceil(e, bought, order.price, PRECISION);
    (sold, bought)
}

/// How much the taker can buy from this order, paying the rounded-up cost.
/// Returns (sold, bought) or (0, 0) if the trade can't be executed
fn buy_amounts_for(e: &Env, order: &Order, buy_left: i128) -> (i128, i128) {
    let bought = buy_left.min(order.amount);
    if bought <= 0 {
        return (0, 0);
    }
    //a cost beyond i128 cannot be paid
    match try_mul_div_ceil(e, bought, order.price, PRECISION) {
        Some(sold) => (sold, bought),
        None => (0, 0),
    }
}

/// Cross an existing taker order against maker orders. The taker order's owner pays the makers
/// through their allowance and is paid exactly at the taker order's price; the spread crossed
/// goes to `trader`. A taker order its owner cannot back or be paid for is skipped untouched.
/// Returns (amount the owner sold, amount the makers delivered, surplus paid to `trader`)
pub(crate) fn cross_orders(
    e: &Env,
    trader: &Address,
    taker_order_id: u128,
    orders: &Vec<u128>,
) -> (i128, i128, i128) {
    //an expired taker order is gone
    let mut taker_order = match load_live_order(e, taker_order_id) {
        Some(o) => o,
        None => e.panic_with_error(OrderbookError::OrderNotFound),
    };
    let axis = e.current_contract_address();
    let owner = taker_order.owner.clone();
    let selling = taker_order.selling.clone();
    let buying = taker_order.buying.clone();
    //the makers deliver to the contract, which pays the owner and the trader out of it: an asset
    //requiring issuer authorization cannot pass through until the issuer authorizes the contract
    if !order::can_receive(e, &buying, &axis) {
        e.panic_with_error(OrderbookError::IntermediaryCannotReceive);
    }
    //the owner backs the taker order with balance and allowance like any maker, and is paid
    //with a plain transfer
    let budget = taker_order.amount.min(order::backing(e, &selling, &owner));
    if budget <= 0 || !order::can_receive(e, &buying, &owner) {
        events::emit_skip(e, taker_order.id);
        return (0, 0, 0);
    }
    //the surplus is paid with a plain transfer too
    order::require_can_receive(e, &buying, trader);
    //on-book orders are sell-equivalent, so cross from the sell side
    let max_exec_price = get_price_threshold(e, &TradeDirection::Sell, taker_order.price);
    let mut dispatcher = Dispatcher::new(
        e,
        owner.clone(),
        selling.clone(),
        buying.clone(),
        axis.clone(),
        false,
        false,
    );
    match_orders(
        e,
        &TradeDirection::Sell,
        budget,
        &selling,
        &buying,
        max_exec_price,
        orders,
        Some(taker_order.price),
        &mut dispatcher,
    );
    let (paid, received, _) = dispatcher.settle();
    if paid == 0 {
        return (0, 0, 0);
    }
    //the owner gets exactly what the taker order asks for, the surplus goes to the trader
    let owed = mul_div_ceil(e, paid, taker_order.price, PRECISION);
    let surplus = received - owed;
    //every fill covers what the owner is owed for it (`profit_floor`), so this cannot happen
    if surplus < 0 {
        e.panic_with_error(OrderbookError::Overflow);
    }
    let buying_token = token::Client::new(e, &buying);
    buying_token.transfer(&axis, &owner, &owed);
    if surplus > 0 {
        buying_token.transfer(&axis, trader, &surplus);
    }
    let mut left = taker_order.amount - paid;
    if is_dust(e, left, taker_order.price) {
        left = 0;
    }
    order::apply_change(e, &mut taker_order, left);
    events::emit_trade(
        e,
        &buying,
        &selling,
        taker_order.id,
        trader,
        &owner,
        owed,
        paid,
        left,
    );
    (paid, received, surplus)
}

/// Load a maker order and pre-validate it for matching.
/// # Returns
/// * `None` when the order is missing, has no remaining amount, has expired, or trades another
///   pair (an id the owner reused for a new order on a different pair)
fn load_matching_order(
    e: &Env,
    order_id: u128,
    taker_selling: &Address,
    taker_buying: &Address,
    now: u64,
) -> Option<Order> {
    let fetched = load_order(e, order_id)?;
    if &fetched.selling != taker_buying || &fetched.buying != taker_selling {
        return None;
    }
    //skip expired and empty orders
    if fetched.amount <= 0 || fetched.is_expired(now) {
        return None;
    }
    Some(fetched)
}
