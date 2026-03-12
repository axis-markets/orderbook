use crate::dispatcher::Dispatcher;
use crate::errors::OrderbookError;
use crate::events::emit_trade;
use crate::order::{load_order, remove_order, update_order, Order};
use crate::settings::PRECISION;
use crate::trade;
use soroban_sdk::{Address, Env, Vec};

pub fn invert_price(e: &Env, price: i128) -> i128 {
    let res = PRECISION * PRECISION / price;
    if res < 0 {
        e.panic_with_error(OrderbookError::Overflow);
    }
    res
}

pub fn execute_orders(
    e: &Env,
    taker: &Address,
    amount: i128,
    selling: &Address,
    buying: &Address,
    max_price: i128,
    orders: Vec<u64>,
    dispatcher: &mut Dispatcher,
) -> (i128, i128) {
    let max_exec_price = invert_price(&e, max_price);
    let now = e.ledger().timestamp();
    let mut total_bought = 0i128;
    let mut total_sold = 0i128;
    let mut amount_left = amount;

    for maker_order_id in orders.iter() {
        //load order from storage
        let fetched_maker_order = load_order(e, &maker_order_id);
        //skip not found orders
        if fetched_maker_order.is_none() {
            continue;
        }
        let mut order = fetched_maker_order.unwrap();
        //make sure that we are trading correct tokens
        if &order.selling != buying || &order.buying != selling {
            e.panic_with_error(OrderbookError::InvalidMatch)
        }
        //skip orders with price worse than requested or expired
        if order.price > max_exec_price || order.amount <= 0 || order.expires < now {
            continue; //TODO: for buy orders the condition will be order.price < max_exec_price
        }
        //calculate maximum amount that can be bought at this price
        let mut bought = order.price * amount_left / PRECISION;
        if bought <= 0 {
            continue; //cannot execute the order
        }
        let mut sold = amount_left;

        if bought >= order.amount {
            //recalculate how much we can take from the order
            if bought > order.amount {
                //set max available amount from the order
                bought = order.amount;
                //recalculate sold amount
                sold = invert_price(e, order.price) * bought / PRECISION;
                if sold <= 0 {
                    continue; //cannot execute the order
                }
            }
        }
        //update maker order amount
        apply_order_trade(e, &mut order, bought);
        //accumulate
        total_bought += bought;
        total_sold += sold;
        amount_left -= sold;
        //check overflows
        if amount_left < 0 {
            //bought more than planned
            e.panic_with_error(OrderbookError::Overflow);
        }
        //create trade descriptor
        trade_with_order(&e, order, taker.clone(), bought, sold, dispatcher);
        //stop iterating if fully executed
        if amount_left == 0 {
            break;
        }
    }
    if total_sold < 0
        || total_bought < 0
        || (total_sold == 0 && total_bought != 0)
        || (total_sold != 0 && total_bought == 0)
    {
        e.panic_with_error(OrderbookError::InvalidMatch);
    }
    (total_sold, total_bought)
}

pub fn cross_orders(
    e: &Env,
    trader: &Address,
    taker_order_id: u64,
    orders: Vec<u64>,
    dispatcher: &mut Dispatcher,
) -> (i128, i128) {
    //load from orderbook
    let fetched_taker_order = load_order(&e, &taker_order_id);
    if fetched_taker_order.is_none() {
        e.panic_with_error(OrderbookError::OrderNotFound);
    }
    let mut taker_order = fetched_taker_order.unwrap();
    //try to fill the order
    let (sold, bought) = execute_orders(
        &e,
        &trader,
        taker_order.amount,
        &taker_order.selling,
        &taker_order.buying,
        taker_order.price,
        orders,
        dispatcher,
    );
    //if the trade was successful
    if sold > 0 {
        //update taker order amount
        apply_order_trade(&e, &mut taker_order, sold);
        //emit event
        trade_with_order(&e, taker_order, trader.clone(), sold, bought, dispatcher);
    }

    //return actual sold/bought amounts
    (sold, bought)
}

fn apply_order_trade(e: &Env, order: &mut Order, bought_from_order: i128) {
    if bought_from_order == order.amount {
        //executed in full - remove from orderbook
        remove_order(e, &order);
        return;
    }
    if bought_from_order > order.amount {
        //attempt to sell more than planned
        e.panic_with_error(OrderbookError::InvalidMatch);
    }
    //executed partially, adjust amount
    order.amount -= bought_from_order;
    //update in orderbook
    update_order(e, &order);
}

fn trade_with_order(
    e: &Env,
    order: Order,
    taker: Address,
    bought_from_order: i128,
    sold_to_order: i128,
    dispatcher: &mut Dispatcher,
) {
    let maker = order.owner;
    //add amounts to settle
    dispatcher.add(&taker, &maker, &order.buying, sold_to_order);
    dispatcher.add(&e.current_contract_address(), &taker, &order.selling, bought_from_order);

    //TODO: settle directly
    //dispatcher.add(&taker, &maker, &order.selling, sold_to_order);
    //dispatcher.add(&maker, &taker, &order.buying, bought_from_order);
    //prepare and emit trade event
    let trade = trade::Trade {
        id: trade::next_trade_id(e),
        order: order.id,
        taker,
        maker,
        sold: sold_to_order,
        bought: bought_from_order,
    };
    emit_trade(&e, order.buying, order.selling, trade);
}
