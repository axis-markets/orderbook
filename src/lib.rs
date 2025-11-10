#![no_std]
mod errors;
mod events;
mod order;
mod orderbook;
mod settings;
mod tests;
mod trade;

use crate::errors::OrderbookError;
use crate::events::emit_trade;
use crate::settings::{bump_instance, get_settings, update_settings, Settings, PRECISION};
use order::{Order, OrderType};
use soroban_sdk::{contract, contractimpl, token, Address, Env, Vec};

#[contract]
pub struct SorobanOrderbook;

#[contractimpl]
impl SorobanOrderbook {
    /// Configure contract settings
    ///
    /// # Arguments
    ///
    /// * `admin` - Admin account address
    /// * `fee` - Trade fee paid by the taker (in ‰)
    ///
    /// # Panics
    ///
    /// Panics if the contract is already initialized
    pub fn configure(e: Env, admin: Address, fee: u32) {
        admin.require_auth();
        let existing = get_settings(&e);
        if existing.is_some() {
            //require auth from the previous admin if settings have been already initialized
            existing.unwrap().admin.require_auth(); //e.panic_with_error(OrderbookError::NotAuthorized);
        }
        let settings = Settings { admin, fee };
        update_settings(&e, &settings);
    }

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
        bump_instance(&e, 1);
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
        //deposit selling tokens to contract
        let token_client = token::Client::new(&e, &selling);
        token_client.transfer(&trader, &e.current_contract_address(), &order_amount);

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
        bump_instance(&e, 1);
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
            let selling_client = token::Client::new(&e, &order_to_remove.selling);
            selling_client.transfer(
                &e.current_contract_address(),
                &trader,
                &order_to_remove.amount,
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
        bump_instance(&e, 1);
        //check amount
        if amount <= 0 {
            e.panic_with_error(OrderbookError::InsufficientBalance)
        }
        //let max_exec_price = invert_price(&e, max_price);
        let this = e.current_contract_address();
        //trade
        let (trades, total_sold, total_bought) =
            orderbook::execute_orders(&e, &trader, amount, &selling, &buying, max_price, orders);

        //init token clients
        let selling_client = token::Client::new(&e, &selling);
        let buying_client = token::Client::new(&e, &buying);

        //transfer sold tokens from trader (taker) to contract
        selling_client.transfer(&trader, &this, &total_sold);
        for trade in trades.iter() {
            //transfer funds from contract to maker
            selling_client.transfer(&this, &trade.maker, &trade.sold);
            //emit event
            emit_trade(&e, trade);
        }
        //transfer bought tokens to trader
        buying_client.transfer(&this, &trader, &total_bought);
        //TODO: consider using try_transfer() instead of transfer() to handle insufficient funds/non-authorized cases
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
        /*//taker order should always be newer than maker orders - not sure about this
        if orders.iter().any(|order_id| order_id > taker_order_id) {
            e.panic_with_error(OrderbookError::InvalidMatch);
        }*/
        //load from orderbook
        let fetched_taker_order = order::load_order(&e, &taker_order_id);
        if fetched_taker_order.is_none() {
            e.panic_with_error(OrderbookError::OrderNotFound);
        }
        let mut taker_order = fetched_taker_order.unwrap();
        //try to fill the order
        let (sold, bought) = SorobanOrderbook::fill(
            e.clone(),
            trader.clone(),
            taker_order.amount,
            taker_order.selling.clone(),
            taker_order.buying.clone(),
            taker_order.price,
            orders,
        );
        //TODO: trader should receive all funds saved from matching existing orders and removing inefficiency
        //update taker order amount
        orderbook::apply_order_trade(&e, &mut taker_order, sold);
        //emit event
        let trade = orderbook::trade_with_order(&e, taker_order, trader, sold, bought);
        emit_trade(&e, trade);
        //return actual sold/bought amounts
        (sold, bought)
    }
}
