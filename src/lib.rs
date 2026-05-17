#![no_std]
mod dispatcher;
mod errors;
mod events;
mod order;
mod orderbook;
mod tests;
mod trade;
mod ttl;
mod utils;

use crate::dispatcher::Dispatcher;
use crate::errors::OrderbookError;
use crate::ttl::bump_instance;
use crate::utils::shorten;
use order::{Order, OrderKind};
use orderbook::{invert_price, PRECISION};
use soroban_sdk::{contract, contractimpl, log, Address, Env, Vec};

#[contract]
pub struct Axis;

#[contractimpl]
impl Axis {
    // Create new contract
    //pub fn __constructor(e: Env) {}

    /// Get lat order id
    ///
    /// # Returns
    ///
    /// Last created order id
    pub fn last(e: Env) -> u64 {
        order::get_last_order_id(&e)
    }

    /// Fetch existing order
    ///
    /// # Arguments
    ///
    /// * `id` - ID of the order to fetch
    ///
    /// # Returns
    ///
    /// Order fetched from the storage
    ///
    /// # Panics
    pub fn order(e: Env, id: u64) -> Option<Order> {
        order::load_order(&e, &id)
    }

    /// Trade with DEX and create sell limit order if quote not executed in full
    ///
    ///  # Arguments
    /// * `kind` - Order type (limit, fill, fill-or-kill)
    /// * `trader` - Trader address
    /// * `amount` - Amount of tokens to sell
    /// * `selling` - Selling token address
    /// * `buying` - Buying token address
    /// * `price` - Price the trader willing to accept
    /// * `orders` - Optional list of order IDs to match before creating the order on-chain
    ///
    /// # Returns
    ///
    /// * Amount of sold tokens
    /// * Amount of bought tokens
    /// * ID of the newly created order if any
    ///
    /// # Panics
    ///
    /// If the trader has insufficient balance
    /// If any of the orders provided do not match selling/buying asset
    /// If the trade causes an overflow
    pub fn sell(
        e: Env,
        kind: OrderKind,
        trader: Address,
        amount: i128,
        selling: Address,
        buying: Address,
        price: i128,
        orders: Vec<u64>,
    ) -> (i128, i128, u64) {
        //need permission from the trader
        trader.require_auth();
        //keep contract alive
        bump_instance(&e);
        let axis = e.current_contract_address();
        let mut sold = 0;
        let mut bought = 0;

        let mut dispatcher = Dispatcher::new(&e);
        //execute orders if list was provided
        if orders.len() > 0 {
            (sold, bought) = orderbook::sell_to_orders(
                &e,
                &trader,
                amount,
                &selling,
                &buying,
                price,
                orders,
                &mut dispatcher,
            );
        }

        //FillOrKill does not allow partial execution
        if kind == OrderKind::FillOrKill && sold < amount {
            return (0, 0, 0);
        }

        //return if executed in full or partial execution requested
        if sold == amount || kind == OrderKind::Fill {
            //settle all payments
            dispatcher.settle();
            return (sold, bought, 0);
        }
        //not executed in full, need to create limit order
        let order_amount = amount - sold;
        //deposit order tokens to contract
        dispatcher.add_transfer(&trader, &axis, &selling, order_amount);

        //add new order to orderbook
        log!(
            &e,
            "create order",
            shorten(&trader),
            shorten(&selling),
            shorten(&buying),
            order_amount,
            price
        );

        let orderid = order::create_order(
            &e,
            OrderKind::Limit,
            trader,
            order_amount,
            selling,
            buying,
            price,
        );

        //settle all payments
        dispatcher.settle();
        //return selling/buying amounts and new order ID
        (sold, bought, orderid)
    }

    /// Trade with DEX and create buy limit order if quote not executed in full
    ///
    ///  # Arguments
    /// * `kind` - Order type (limit, fill, fill-or-kill)
    /// * `trader` - Trader address
    /// * `amount` - Amount of tokens to buy
    /// * `buying` - Buying token address (what the trader receives)
    /// * `selling` - Selling token address (what the trader pays with)
    /// * `price` - Maximum price (selling tokens per 1 buying token, PRECISION fixed-point) the trader is willing to pay
    /// * `orders` - Optional list of order IDs to match before creating the order on-chain
    ///
    /// # Returns
    ///
    /// * Amount of sold tokens
    /// * Amount of bought tokens
    /// * ID of the newly created order if any
    ///
    /// # Panics
    ///
    /// If the trader has insufficient balance
    /// If any of the orders provided do not match selling/buying asset
    /// If the trade causes an overflow
    pub fn buy(
        e: Env,
        kind: OrderKind,
        trader: Address,
        amount: i128,
        buying: Address,
        selling: Address,
        price: i128,
        orders: Vec<u64>,
    ) -> (i128, i128, u64) {
        //need permission from the trader
        trader.require_auth();
        //keep contract alive
        bump_instance(&e);
        let axis = e.current_contract_address();
        let mut sold = 0;
        let mut bought = 0;

        let mut dispatcher = Dispatcher::new(&e);
        //execute orders if list was provided
        if orders.len() > 0 {
            (sold, bought) = orderbook::buy_from_orders(
                &e,
                &trader,
                amount,
                &buying,
                &selling,
                price,
                orders,
                &mut dispatcher,
            );
        }

        //FillOrKill does not allow partial execution
        if kind == OrderKind::FillOrKill && bought < amount {
            return (0, 0, 0);
        }

        //return if executed in full or partial execution requested
        if bought == amount || kind == OrderKind::Fill {
            //settle all payments
            dispatcher.settle();
            return (sold, bought, 0);
        }
        //not executed in full, need to create limit order for the remaining buy amount
        let remaining_buy = amount - bought;
        //worst-case deposit covering the remaining buy amount at the max price
        let order_deposit = remaining_buy * price / PRECISION;
        //deposit selling tokens to contract to fund future fills
        dispatcher.add_transfer(&trader, &axis, &selling, order_deposit);

        //store the remainder as a sell-equivalent order: the buyer offers `order_deposit`
        //of `selling` tokens at price `invert(price)` (buying per selling)
        let internal_price = invert_price(&e, price);

        //add new order to orderbook
        log!(
            &e,
            "create order",
            shorten(&trader),
            shorten(&selling),
            shorten(&buying),
            order_deposit,
            internal_price
        );

        let orderid = order::create_order(
            &e,
            OrderKind::Limit,
            trader,
            order_deposit,
            selling,
            buying,
            internal_price,
        );

        //settle all payments
        dispatcher.settle();
        //return selling/buying amounts and new order ID
        (sold, bought, orderid)
    }

    /// Cancel existing order
    ///
    /// # Arguments
    ///
    /// * `id` - ID of the order to cancel
    /// * `trader` - Trader address
    ///
    /// # Returns
    ///
    /// Order fetched from the storage
    ///
    /// # Panics
    ///
    /// If trader is not the owner of the order
    pub fn cancel(e: Env, id: u64, trader: Address) {
        //TODO: add "replace" function that cancels an active order and creates a new one atomically
        //TODO: allow canceling more than one order at a time
        trader.require_auth();
        bump_instance(&e);
        //fetch order from the book
        let order = order::load_order(&e, &id);
        //only if it still exists
        if !order.is_none() {
            let order_to_remove = order.unwrap();
            //only owner can cancel
            if order_to_remove.owner != trader {
                e.panic_with_error(OrderbookError::NotAuthorized)
            }
            //return unsold tokens to the trader
            Dispatcher::transfer(
                &e,
                &e.current_contract_address(),
                &trader,
                &order_to_remove.selling,
                order_to_remove.amount,
            );
            //remove order from the book
            order::remove_order(&e, &order_to_remove);
        }
    }

    /// Fill existing orders using another matching order from the orderbook
    ///
    ///  # Arguments
    /// * `trader` - Trader address
    /// * `taker_order_id` - ID of the order that serves as a taker
    /// * `orders` - List of order IDs to match before creating the order on chain
    ///
    /// # Returns
    ///
    /// * Amount of sold tokens
    /// * Amount of bought tokens
    ///
    /// # Panics
    ///
    /// If the taker order was not found
    /// If any of the orders provided do not match selling/buying asset
    /// If the trade causes an overflow
    pub fn fill_order(
        e: Env,
        trader: Address,
        taker_order_id: u64,
        orders: Vec<u64>,
    ) -> (i128, i128) {
        //TODO: trader should receive all profits saved from matching existing orders and removing inefficiency
        let mut dispatcher = Dispatcher::new(&e);
        let (sold, bought) =
            orderbook::cross_orders(&e, &trader, taker_order_id, orders, &mut dispatcher);
        dispatcher.settle();
        (sold, bought)
    }
}
