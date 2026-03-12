#![no_std]
mod dispatcher;
mod errors;
mod events;
mod order;
mod orderbook;
mod settings;
mod tests;
mod trade;
mod utils;

use crate::dispatcher::Dispatcher;
use crate::errors::OrderbookError;
use crate::settings::{bump_instance, PRECISION};
use crate::utils::shorten;
use order::{Order, OrderType};
use soroban_sdk::{contract, contractimpl, log, Address, Env, Vec};

#[contract]
pub struct SorobanOrderbook;

#[contractimpl]
impl SorobanOrderbook {
    /// Create new contract
    pub fn __constructor(e: Env) {}

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
    /// * `trader` - Trader address
    /// * `amount` - Amount of tokens to sell
    /// * `selling` - Selling token address
    /// * `buying` - Buying token address
    /// * `price` - Min price a trader willing to accept
    /// * `ttl` - Time to live for an order (expired orders will be automatically purged)
    /// * `orders` - Optional list of order IDs to match before creating the order on chain
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
    pub fn sell_limit(
        e: Env,
        trader: Address,
        amount: i128,
        selling: Address,
        buying: Address,
        price: i128,
        ttl: u64, //time to live in seconds
        orders: Vec<u64>,
    ) -> (i128, i128, u64) {
        trader.require_auth();
        bump_instance(&e);
        // TODO: check deposit min amount
        //let deposit: i128 = amount * max_price / PRECISION;
        let mut order_amount = amount;
        let mut sold = 0;
        let mut bought = 0;

        if orders.len() > 0 {
            (sold, bought) = SorobanOrderbook::fill(
                e.clone(),
                trader.clone(),
                amount,
                selling.clone(),
                buying.clone(),
                price,
                orders,
            );
            if sold == amount {
                return (sold, bought, 0); //fully executed
            }
            if sold > 0 {
                order_amount = amount - sold; //partially executed
            }
        }
        log!(
            &e,
            "create order",
            shorten(&trader),
            shorten(&selling),
            shorten(&buying),
            order_amount,
            price
        );
        //deposit selling tokens to contract
        Dispatcher::transfer(
            &e,
            &trader,
            &e.current_contract_address(),
            &selling,
            order_amount,
        );

        //add new order to orderbook
        let orderid = order::create_order(
            &e,
            OrderType::Limit,
            trader,
            order_amount,
            selling,
            buying,
            price,
            ttl,
        );
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

    /// Trade with orders
    ///
    ///  # Arguments
    /// * `trader` - Trader address
    /// * `amount` - Amount of tokens to sell
    /// * `selling` - Selling token address
    /// * `buying` - Buying token address
    /// * `max_price` - Max price a trader willing to pay
    /// * `ttl` - Time to live for an order (expired orders will be automatically purged)
    /// * `orders` - List of order IDs to match before creating the order on chain
    ///
    /// # Returns
    ///
    /// * Amount of sold tokens
    /// * Amount of bought tokens
    ///
    /// # Panics
    ///
    /// If any of the orders provided do not match selling/buying asset
    /// If the trade causes an overflow
    pub fn fill(
        e: Env,
        trader: Address,
        amount: i128,
        selling: Address,
        buying: Address,
        max_price: i128,
        orders: Vec<u64>,
    ) -> (i128, i128) {
        trader.require_auth();
        bump_instance(&e);
        //check amount
        if amount <= 0 {
            e.panic_with_error(OrderbookError::InsufficientBalance)
        }
        //let max_exec_price = invert_price(&e, max_price);
        let mut dispatcher = Dispatcher::new(&e);
        //trade
        let (total_sold, total_bought) = orderbook::execute_orders(
            &e,
            &trader,
            amount,
            &selling,
            &buying,
            max_price,
            orders,
            &mut dispatcher,
        );
        dispatcher.settle();
        /*//init token clients
        let selling_client = token::Client::new(&e, &selling);
        let buying_client = token::Client::new(&e, &buying);

        //transfer sold tokens from trader (taker) to contract
        selling_client.transfer(&trader, &this, &total_sold);
        for trade in trades.iter() {
            //transfer funds from contract to maker
            selling_client.transfer(&this, &trade.maker, &trade.sold);
        }
        //transfer bought tokens to trader
        buying_client.transfer(&this, &trader, &total_bought);*/
        //return fill result
        (total_sold, total_bought)
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
