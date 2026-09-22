use super::setup::{
    actor, assert_no_custody, balance, code, fake_asset, fund, register_axis, setup_test,
    store_order, try_trade,
};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, trade::TradeStep, AxisClient};
use soroban_sdk::{Address, Env, Vec};

// Cross rates:
//   EUR/USD = 1.2  (1 EUR costs 1.2 USD)
//   GBP/USD = 1.32 (1 GBP costs 1.32 USD)  =>  GBP/EUR = 1.32 / 1.2 = 1.1
const EUR_USD: i128 = 12 * PRECISION / 10; // 1.2 USD per EUR
const GBP_EUR: i128 = 11 * PRECISION / 10; // 1.1 EUR per GBP

/// Set up the USD/EUR and EUR/GBP markets used by the two-hop tests.
/// `maker1` sells `eur_amount` EUR for USD at 1.2; `maker2` sells `gbp_amount` GBP for EUR at 1.1.
/// Returns (client, contract_address, maker1, maker2, order1, order2).
fn two_market_setup<'a>(
    e: &'a Env,
    usd: &Address,
    eur: &Address,
    gbp: &Address,
    eur_amount: i128,
    gbp_amount: i128,
) -> (AxisClient<'a>, Address, Address, Address, u128, u128) {
    let contract_address = register_axis(e);
    let client = AxisClient::new(e, &contract_address);

    let maker1 = actor(e);
    let maker2 = actor(e);
    fund(e, eur, &contract_address, &maker1, eur_amount);
    fund(e, gbp, &contract_address, &maker2, gbp_amount);

    let order1 = store_order(&client, &maker1, eur_amount, eur, usd, EUR_USD);
    let order2 = store_order(&client, &maker2, gbp_amount, gbp, eur, GBP_EUR);
    (client, contract_address, maker1, maker2, order1, order2)
}

fn two_hop_path(
    e: &Env,
    eur: &Address,
    gbp: &Address,
    order1: u128,
    order2: u128,
) -> Vec<TradeStep> {
    Vec::from_array(
        e,
        [
            TradeStep {
                asset: eur.clone(),
                orders: Vec::from_array(e, [order1]),
            },
            TradeStep {
                asset: gbp.clone(),
                orders: Vec::from_array(e, [order2]),
            },
        ],
    )
}

#[test]
fn test_swap_sell_two_hops() {
    // Sell 1000 USD -> EUR -> GBP.
    //   1000 USD / 1.2 = 833 EUR (floor), 833 EUR / 1.1 = 757 GBP (floor)
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, maker1, maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 833, 757);
    fund(&e, &usd, &contract_address, &trader, 1000);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &757,
        &path,
        &None,
    );

    assert_eq!(sold, 1000);
    assert_eq!(bought, 757);

    // trader: spent all USD, received GBP, no EUR residue
    assert_eq!(balance(&e, &usd, &trader), 0);
    assert_eq!(balance(&e, &gbp, &trader), 757);
    assert_eq!(balance(&e, &eur, &trader), 0);

    // makers received their proceeds; both orders fully consumed
    assert_eq!(balance(&e, &usd, &maker1), 1000);
    assert_eq!(balance(&e, &eur, &maker1), 0);
    assert_eq!(balance(&e, &eur, &maker2), 833);
    assert_eq!(balance(&e, &gbp, &maker2), 0);
    assert!(client.order(&order1).is_none());
    assert!(client.order(&order2).is_none());

    // contract nets to zero across every asset
    assert_no_custody(&e, &contract_address, &[&usd, &eur, &gbp]);
}

#[test]
fn test_swap_buy_two_hops() {
    // Buy exactly 100 GBP via EUR using USD, spending at most 200 USD.
    //   100 GBP * 1.1 = 110 EUR, 110 EUR * 1.2 = 132 USD
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, maker1, maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 110, 100);
    fund(&e, &usd, &contract_address, &trader, 200);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    let (sold, bought) = client.swap(
        &TradeDirection::Buy,
        &trader,
        &usd,
        &200,
        &100,
        &path,
        &None,
    );

    assert_eq!(bought, 100);
    assert_eq!(sold, 132);

    // trader keeps the unused 68 USD
    assert_eq!(balance(&e, &usd, &trader), 68);
    assert_eq!(balance(&e, &gbp, &trader), 100);
    assert_eq!(balance(&e, &eur, &trader), 0);

    assert_eq!(balance(&e, &usd, &maker1), 132);
    assert_eq!(balance(&e, &eur, &maker2), 110);
    assert!(client.order(&order1).is_none());
    assert!(client.order(&order2).is_none());
    assert_no_custody(&e, &contract_address, &[&usd, &eur, &gbp]);
}

#[test]
fn test_swap_sell_slippage_kill() {
    // Same liquidity as the happy path, but demand more GBP than the route yields.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, _maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 833, 757);
    fund(&e, &usd, &contract_address, &trader, 1000);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    // route yields 757 GBP but we require at least 758 -> NotFilled
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &1000,
            &758,
            &path,
            &None
        )),
        Some(709)
    );

    // nothing moved, no order touched
    assert_eq!(balance(&e, &usd, &trader), 1000);
    assert_eq!(balance(&e, &gbp, &trader), 0);
    assert_no_custody(&e, &contract_address, &[&usd, &eur, &gbp]);
    assert_eq!(client.order(&order1).unwrap().amount, 833);
    assert_eq!(client.order(&order2).unwrap().amount, 757);
}

#[test]
fn test_swap_buy_slippage_kill() {
    // Buy 100 GBP would cost 132 USD, but cap spending at 131 -> NotFilled.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, _maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 110, 100);
    fund(&e, &usd, &contract_address, &trader, 200);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Buy,
            &trader,
            &usd,
            &131,
            &100,
            &path,
            &None
        )),
        Some(709)
    );
    assert_eq!(balance(&e, &usd, &trader), 200);
    assert_eq!(balance(&e, &gbp, &trader), 0);
    assert_eq!(client.order(&order1).unwrap().amount, 110);
    assert_eq!(client.order(&order2).unwrap().amount, 100);
}

#[test]
fn test_swap_sell_insufficient_liquidity_kill() {
    // Second hop has only 100 GBP of liquidity, far short of the 757 the route needs,
    // so the first hop's full EUR output cannot be sold onward -> NotFilled.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, _maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 833, 100);
    fund(&e, &usd, &contract_address, &trader, 1000);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &1000,
            &1,
            &path,
            &None
        )),
        Some(709)
    );
    assert_eq!(balance(&e, &usd, &trader), 1000);
    assert_eq!(balance(&e, &gbp, &trader), 0);
    assert_no_custody(&e, &contract_address, &[&usd, &eur, &gbp]);
    assert_eq!(client.order(&order1).unwrap().amount, 833);
    assert_eq!(client.order(&order2).unwrap().amount, 100);
}

#[test]
fn test_swap_unbacked_maker_kills_the_route() {
    // A maker of the second hop lost their backing: the swap is strict and fails instead of
    // leaving the contract holding the first hop's EUR
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 833, 757);
    fund(&e, &usd, &contract_address, &trader, 1000);
    // maker2 moves the GBP away
    soroban_sdk::token::Client::new(&e, &gbp).transfer(&maker2, &trader, &757);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    assert!(client
        .try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &1000,
            &757,
            &path,
            &None
        )
        .is_err());
    assert_eq!(balance(&e, &usd, &trader), 1000);
    assert_no_custody(&e, &contract_address, &[&usd, &eur, &gbp]);
    assert_eq!(client.order(&order1).unwrap().amount, 833);
    assert_eq!(client.order(&order2).unwrap().amount, 757);
}

#[test]
fn test_swap_single_hop_like_fill_or_kill() {
    // A one-step path behaves like a FillOrKill trade in a single market.
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let maker = actor(&e);
    fund(&e, &usd, &contract_address, &trader, 1000);
    fund(&e, &eur, &contract_address, &maker, 833);
    let order1 = store_order(&client, &maker, 833, &eur, &usd, EUR_USD);

    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: eur.clone(),
            orders: Vec::from_array(&e, [order1]),
        }],
    );
    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &833,
        &path,
        &None,
    );

    assert_eq!(sold, 1000);
    assert_eq!(bought, 833);
    assert_eq!(balance(&e, &usd, &trader), 0);
    assert_eq!(balance(&e, &eur, &trader), 833);
    assert_eq!(balance(&e, &usd, &maker), 1000);
    assert!(client.order(&order1).is_none());
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test]
#[should_panic(expected = "#704")]
fn test_swap_wrong_asset_in_step_panics() {
    // The step claims to buy GBP but points at a USD/EUR order -> InvalidMatch (704).
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let maker = actor(&e);
    fund(&e, &usd, &contract_address, &trader, 1000);
    fund(&e, &eur, &contract_address, &maker, 1000);
    let order1 = store_order(&client, &maker, 833, &eur, &usd, EUR_USD);

    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: gbp.clone(),
            orders: Vec::from_array(&e, [order1]),
        }],
    );
    client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &1,
        &path,
        &None,
    );
}

#[test]
fn test_fill_or_kill_leaves_no_trace() {
    // An unfilled FillOrKill fails the call: no event, no transfer, no order change, even
    // though it partially matched while computing the fill.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);

    let taker = actor(&e);
    fund(&e, &usd, &contract_address, &maker, 10000);
    fund(&e, &eur, &contract_address, &taker, 10000);

    // maker sells only 300 USD wanting 1.2 EUR per USD
    let order_id = store_order(&client, &maker, 300, &usd, &eur, EUR_USD);

    // taker FillOrKill: sell 1000 EUR for USD, but only 300 USD of liquidity exists
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::FillOrKill,
            &taker,
            1000,
            &eur,
            &usd,
            PRECISION / 2,
            &Vec::from_array(&e, [order_id]),
        ),
        Some(709)
    );
    assert_eq!(balance(&e, &eur, &taker), 10000);
    assert_eq!(balance(&e, &usd, &taker), 0);
    assert_eq!(client.order(&order_id).unwrap().amount, 300);
}
