use super::setup::{
    actor, approve, assert_no_custody, assert_trades_match_balances, authorize, balance, balances,
    code, fake_asset, fake_asset_auth_required, fund, list_asset, register_axis, setup_test,
    store_order, try_trade, UNIT_PRICE,
};
use crate::events::{SwapEvent, TradeEvent};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, trade::TradeStep, AxisClient};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token, Address, Env, Event, Vec};

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
#[should_panic(expected = "#709")]
fn test_swap_wrong_asset_in_step_is_skipped() {
    // The step claims to buy GBP but points at a USD/EUR order: the order is skipped, the hop
    // fills nothing -> NotFilled (709).
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

#[test]
fn test_swap_routes_around_an_unbacked_maker() {
    // The hop lists a maker who revoked the allowance first and a backed maker second: the
    // swap completes from the backed maker and the unbacked order stays untouched
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = register_axis(&e);
    let client = AxisClient::new(&e, &contract_address);
    let unbacked = actor(&e);
    let backed = actor(&e);
    fund(&e, &eur, &contract_address, &unbacked, 833);
    fund(&e, &eur, &contract_address, &backed, 833);
    fund(&e, &usd, &contract_address, &trader, 1000);
    let unbacked_order = store_order(&client, &unbacked, 833, &eur, &usd, EUR_USD);
    let backed_order = store_order(&client, &backed, 833, &eur, &usd, EUR_USD);
    approve(&e, &eur, &contract_address, &unbacked, 0);

    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: eur.clone(),
            orders: Vec::from_array(&e, [unbacked_order, backed_order]),
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
    assert_eq!((sold, bought), (1000, 833));
    assert_eq!(balance(&e, &eur, &trader), 833);
    assert_eq!(client.order(&unbacked_order).unwrap().amount, 833);
    assert!(client.order(&backed_order).is_none());
    assert_no_custody(&e, &contract_address, &[&usd, &eur]);
}

#[test]
fn test_swap_through_an_auth_required_asset_needs_the_contract_authorized() {
    // USD -> R -> GBP, where the issuer of R must authorize every holder. The contract holds R
    // between the hops, and a single hop into R hands it to the trader in one transfer from the
    // contract, so both fail up front with their own code until the issuer authorizes the contract
    let (e, trader, issuer, usd, _) = setup_test();
    let regulated = fake_asset_auth_required(&e, &issuer);
    let gbp = fake_asset(&e, &issuer);
    list_asset(&e, &regulated, UNIT_PRICE);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker1 = actor(&e);
    let maker2 = actor(&e);
    for holder in [&trader, &maker1, &maker2] {
        authorize(&e, &regulated, holder);
    }
    fund(&e, &regulated, &axis, &maker1, 2000);
    fund(&e, &gbp, &axis, &maker2, 1000);
    fund(&e, &usd, &axis, &trader, 2000);
    let order1 = store_order(&client, &maker1, 2000, &regulated, &usd, PRECISION);
    let order2 = store_order(&client, &maker2, 1000, &gbp, &regulated, PRECISION);
    let path = two_hop_path(&e, &regulated, &gbp, order1, order2);
    let swap = |path: &Vec<TradeStep>| {
        client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &1000,
            &1000,
            path,
            &None,
        )
    };

    assert_eq!(code(swap(&path)), Some(712));
    assert_eq!(balance(&e, &usd, &trader), 2000);
    assert_eq!(client.order(&order1).unwrap().amount, 2000);

    let single_hop = Vec::from_array(
        &e,
        [TradeStep {
            asset: regulated.clone(),
            orders: Vec::from_array(&e, [order1]),
        }],
    );
    assert_eq!(code(swap(&single_hop)), Some(712));

    authorize(&e, &regulated, &axis);
    assert_eq!(swap(&single_hop), Ok(Ok((1000, 1000))));
    assert_eq!(balance(&e, &regulated, &trader), 1000);
    assert_eq!(swap(&path), Ok(Ok((1000, 1000))));
    assert_eq!(balance(&e, &gbp, &trader), 1000);
    assert_eq!(balance(&e, &regulated, &maker2), 1000);
    assert!(client.order(&order1).is_none());
    assert_no_custody(&e, &axis, &[&usd, &regulated, &gbp]);
}

/// USD -> EUR -> USD -> EUR route listing `eur_order` in the first and the last hop
fn cyclic_path(
    e: &Env,
    usd: &Address,
    eur: &Address,
    eur_order: u128,
    usd_order: u128,
) -> Vec<TradeStep> {
    let step = |asset: &Address, order: u128| TradeStep {
        asset: asset.clone(),
        orders: Vec::from_array(e, [order]),
    };
    Vec::from_array(
        e,
        [
            step(eur, eur_order),
            step(usd, usd_order),
            step(eur, eur_order),
        ],
    )
}

#[test]
fn test_swap_cyclic_path_reuses_a_maker_order() {
    // The plan reads every hop against the book as it is, the execution against the book the
    // earlier hops left behind. With room for both hops in the shared EUR order the route settles
    // exactly as planned; otherwise the last hop settles short, the swap fails `NotFilled` and
    // nothing moves. The contract keeps nothing either way
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let eur_maker = actor(&e);
    let usd_maker = actor(&e);
    fund(&e, &eur, &axis, &eur_maker, 250);
    fund(&e, &usd, &axis, &usd_maker, 1000);
    fund(&e, &usd, &axis, &trader, 1000);
    let eur_order = store_order(&client, &eur_maker, 250, &eur, &usd, PRECISION);
    let usd_order = store_order(&client, &usd_maker, 1000, &usd, &eur, PRECISION);
    let path = cyclic_path(&e, &usd, &eur, eur_order, usd_order);
    let swap = |amount: i128| {
        client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &amount,
            &amount,
            &path,
            &None,
        )
    };

    // the EUR order delivers 100 in the first hop and 100 more in the last one
    assert_eq!(swap(100), Ok(Ok((100, 100))));
    assert_eq!(client.order(&eur_order).unwrap().amount, 50);
    assert_eq!(client.order(&usd_order).unwrap().amount, 900);
    assert_eq!(balance(&e, &usd, &trader), 900);
    assert_eq!(balance(&e, &eur, &trader), 100);
    assert_eq!(balance(&e, &usd, &eur_maker), 200);
    assert_no_custody(&e, &axis, &[&usd, &eur]);

    // 50 EUR are left: each hop fits on its own, both together do not
    assert_eq!(code(swap(40)), Some(709));
    assert_eq!(client.order(&eur_order).unwrap().amount, 50);
    assert_eq!(balance(&e, &usd, &trader), 900);
    assert_eq!(balance(&e, &eur, &trader), 100);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

#[test]
fn test_swap_cyclic_path_respects_the_maker_backing() {
    // The shared EUR order is large, but its maker holds only 150 EUR: the plan admits each hop on
    // its own, the last hop finds the backing spent by the first one and the swap fails
    let (e, trader, _, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let eur_maker = actor(&e);
    let usd_maker = actor(&e);
    fund(&e, &eur, &axis, &eur_maker, 1000);
    fund(&e, &usd, &axis, &usd_maker, 1000);
    fund(&e, &usd, &axis, &trader, 1000);
    let eur_order = store_order(&client, &eur_maker, 1000, &eur, &usd, PRECISION);
    let usd_order = store_order(&client, &usd_maker, 1000, &usd, &eur, PRECISION);
    soroban_sdk::token::Client::new(&e, &eur).transfer(&eur_maker, &usd_maker, &850);

    let path = cyclic_path(&e, &usd, &eur, eur_order, usd_order);
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &100,
            &100,
            &path,
            &None
        )),
        Some(709)
    );
    assert_eq!(client.order(&eur_order).unwrap().amount, 1000);
    assert_eq!(balance(&e, &usd, &trader), 1000);
    assert_eq!(balance(&e, &eur, &eur_maker), 150);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

/// Sell 1000 USD -> EUR -> GBP with the trader owning the maker order of hop `trader_hop` and
/// another maker the other one. Both orders hold 1000, the route fills 833 EUR and 757 GBP
fn swap_with_the_trader_as_maker(trader_hop: u32) {
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    let (eur_owner, gbp_owner) = match trader_hop {
        0 => (&trader, &maker),
        _ => (&maker, &trader),
    };
    fund(&e, &usd, &axis, &trader, 1000);
    fund(&e, &eur, &axis, eur_owner, 1000);
    fund(&e, &gbp, &axis, gbp_owner, 1000);
    let eur_order = store_order(&client, eur_owner, 1000, &eur, &usd, EUR_USD);
    let gbp_order = store_order(&client, gbp_owner, 1000, &gbp, &eur, GBP_EUR);
    // finite allowances, so what the swap spends of them shows
    let (owned, filled) = match trader_hop {
        0 => (&eur, 833),
        _ => (&gbp, 757),
    };
    approve(&e, &usd, &axis, &trader, 5000);
    approve(&e, owned, &axis, &trader, 5000);
    let allowance =
        |asset: &Address| soroban_sdk::token::Client::new(&e, asset).allowance(&trader, &axis);

    let path = two_hop_path(&e, &eur, &gbp, eur_order, gbp_order);
    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &757,
        &path,
        &None,
    );
    assert_eq!((sold, bought), (1000, 757));

    // each order shrinks by exactly what its owner's allowance paid out
    assert_eq!(client.order(&eur_order).unwrap().amount, 1000 - 833);
    assert_eq!(client.order(&gbp_order).unwrap().amount, 1000 - 757);
    assert_eq!(allowance(owned), 5000 - filled);
    // the route input is paid once, to the trader's own order on the first hop too
    assert_eq!(allowance(&usd), 5000 - 1000);

    // (USD, EUR, GBP) of the trader and of the other maker
    let holdings = |account: &Address| {
        (
            balance(&e, &usd, account),
            balance(&e, &eur, account),
            balance(&e, &gbp, account),
        )
    };
    match trader_hop {
        // the trader pays its own EUR order and sells 833 of its EUR down the route
        0 => {
            assert_eq!(holdings(&trader), (1000, 167, 757));
            assert_eq!(holdings(&maker), (0, 833, 243));
        }
        // the contract pays the trader's GBP order the 833 EUR the first hop delivered
        _ => {
            assert_eq!(holdings(&trader), (0, 833, 1000));
            assert_eq!(holdings(&maker), (1000, 167, 0));
        }
    }
    assert_no_custody(&e, &axis, &[&usd, &eur, &gbp]);
}

#[test]
fn test_swap_trader_owns_a_hop_order() {
    // The trader is the maker of the first hop (paying itself from its own allowance) or of the
    // last one (paid out of the contract): either way its order and allowance move by the fill only
    swap_with_the_trader_as_maker(0);
    swap_with_the_trader_as_maker(1);
}

// Closed route USD -> EUR -> GBP -> USD, priced so that 1000 USD come back as 1280:
//   1000 USD / 1.25 = 800 EUR, 800 EUR / 1.25 = 640 GBP, 640 GBP / 0.5 = 1280 USD
const LOOP_EUR_USD: i128 = 5 * PRECISION / 4; // 1.25 USD per EUR
const LOOP_GBP_EUR: i128 = 5 * PRECISION / 4; // 1.25 EUR per GBP
const LOOP_USD_GBP: i128 = PRECISION / 2; // 0.5 GBP per USD

/// AXIS with the three markets of the closed route, one maker order each, every one larger than
/// the route takes (1000 EUR, 1000 GBP, 2000 USD). The trader holds 1000 USD and lets the
/// contract spend up to 5000 USD
struct ClosedLoop<'a> {
    client: AxisClient<'a>,
    axis: Address,
    /// Makers selling EUR, GBP and USD
    makers: [Address; 3],
    /// Their orders, in route order
    orders: [u128; 3],
    path: Vec<TradeStep>,
}

fn closed_loop<'a>(
    e: &'a Env,
    trader: &Address,
    usd: &Address,
    eur: &Address,
    gbp: &Address,
) -> ClosedLoop<'a> {
    let axis = register_axis(e);
    let client = AxisClient::new(e, &axis);
    let makers = [actor(e), actor(e), actor(e)];
    fund(e, eur, &axis, &makers[0], 1000);
    fund(e, gbp, &axis, &makers[1], 1000);
    fund(e, usd, &axis, &makers[2], 2000);
    let orders = [
        store_order(&client, &makers[0], 1000, eur, usd, LOOP_EUR_USD),
        store_order(&client, &makers[1], 1000, gbp, eur, LOOP_GBP_EUR),
        store_order(&client, &makers[2], 2000, usd, gbp, LOOP_USD_GBP),
    ];
    fund(e, usd, &axis, trader, 1000);
    // a finite allowance, so what the swap spends of it shows
    approve(e, usd, &axis, trader, 5000);
    let step = |asset: &Address, order: u128| TradeStep {
        asset: asset.clone(),
        orders: Vec::from_array(e, [order]),
    };
    let path = Vec::from_array(
        e,
        [
            step(eur, orders[0]),
            step(gbp, orders[1]),
            step(usd, orders[2]),
        ],
    );
    ClosedLoop {
        client,
        axis,
        makers,
        orders,
        path,
    }
}

#[test]
fn test_swap_closed_loop_sell() {
    // The route ends in the asset it starts from. The trader pays the EUR maker 1000 USD out of
    // the allowance, the contract pays the GBP and USD makers out of what the previous hop
    // delivered and sends the 1280 USD of the last hop to the trader in one transfer: the trader
    // nets 280 USD, the allowance drops by the 1000 sold only
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let l = closed_loop(&e, &trader, &usd, &eur, &gbp);
    let [eur_maker, gbp_maker, usd_maker] = &l.makers;
    let accounts = [&trader, eur_maker, gbp_maker, usd_maker, &l.axis];
    let assets = [&usd, &eur, &gbp];
    let before = balances(&e, &accounts, &assets);

    let res = l.client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &1280,
        &l.path,
        &None,
    );
    let events = e.events().all().filter_by_contract(&l.axis);
    assert_eq!(res, (1000, 1280));

    let fill = |order: u128, maker: &Address, pay: &Address, get: &Address, sold, bought, left| {
        TradeEvent {
            selling: pay.clone(),
            buying: get.clone(),
            order,
            taker: trader.clone(),
            maker: maker.clone(),
            sold,
            bought,
            left,
        }
    };
    let fills = [
        fill(l.orders[0], eur_maker, &usd, &eur, 1000, 800, 200),
        fill(l.orders[1], gbp_maker, &eur, &gbp, 800, 640, 360),
        fill(l.orders[2], usd_maker, &gbp, &usd, 640, 1280, 720),
    ];
    let swap = SwapEvent {
        selling: usd.clone(),
        buying: usd.clone(),
        trader: trader.clone(),
        sold: 1000,
        bought: 1280,
    };
    assert_eq!(
        events,
        [
            fills[0].to_xdr(&e, &l.axis),
            fills[1].to_xdr(&e, &l.axis),
            fills[2].to_xdr(&e, &l.axis),
            swap.to_xdr(&e, &l.axis),
        ]
    );
    assert_trades_match_balances(&e, &fills, &accounts, &assets, &before);

    assert_eq!(balance(&e, &usd, &trader), 1280);
    assert_eq!(balance(&e, &eur, &trader), 0);
    assert_eq!(balance(&e, &gbp, &trader), 0);
    assert_eq!(
        token::Client::new(&e, &usd).allowance(&trader, &l.axis),
        5000 - 1000
    );
    assert_eq!(l.client.order(&l.orders[0]).unwrap().amount, 200);
    assert_eq!(l.client.order(&l.orders[1]).unwrap().amount, 360);
    assert_eq!(l.client.order(&l.orders[2]).unwrap().amount, 720);
    assert_no_custody(&e, &l.axis, &assets);
}

#[test]
fn test_swap_closed_loop_buy() {
    // Buy exactly 640 USD spending at most 1000: planned back to front, 640 USD cost 320 GBP,
    // 320 GBP cost 400 EUR, 400 EUR cost 500 USD. The trader keeps the 500 USD it did not sell
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let l = closed_loop(&e, &trader, &usd, &eur, &gbp);

    let res = l.client.swap(
        &TradeDirection::Buy,
        &trader,
        &usd,
        &1000,
        &640,
        &l.path,
        &None,
    );
    assert_eq!(res, (500, 640));
    assert_eq!(balance(&e, &usd, &trader), 1000 - 500 + 640);
    assert_eq!(balance(&e, &eur, &trader), 0);
    assert_eq!(balance(&e, &gbp, &trader), 0);
    assert_eq!(
        token::Client::new(&e, &usd).allowance(&trader, &l.axis),
        5000 - 500
    );
    assert_eq!(balance(&e, &usd, &l.makers[0]), 500);
    assert_eq!(balance(&e, &eur, &l.makers[1]), 400);
    assert_eq!(balance(&e, &gbp, &l.makers[2]), 320);
    assert_eq!(l.client.order(&l.orders[0]).unwrap().amount, 1000 - 400);
    assert_eq!(l.client.order(&l.orders[1]).unwrap().amount, 1000 - 320);
    assert_eq!(l.client.order(&l.orders[2]).unwrap().amount, 2000 - 640);
    assert_no_custody(&e, &l.axis, &[&usd, &eur, &gbp]);
}

#[test]
fn test_swap_closed_loop_kills_and_moves_nothing() {
    // A route returning less than the minimum (`Sell`) or costing more than the maximum (`Buy`)
    // fails `NotFilled`. The output never funds the input: the trader pays the first hop before
    // the last one pays out, so 999 USD cannot start the route that would return 1280
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let l = closed_loop(&e, &trader, &usd, &eur, &gbp);
    let [eur_maker, gbp_maker, usd_maker] = &l.makers;
    let accounts = [&trader, eur_maker, gbp_maker, usd_maker, &l.axis];
    let assets = [&usd, &eur, &gbp];
    let swap = |direction: TradeDirection, selling_amount: i128, buying_amount: i128| {
        code(l.client.try_swap(
            &direction,
            &trader,
            &usd,
            &selling_amount,
            &buying_amount,
            &l.path,
            &None,
        ))
    };
    let before = balances(&e, &accounts, &assets);
    let untouched = || {
        assert_eq!(balances(&e, &accounts, &assets), before);
        assert_eq!(l.client.order(&l.orders[0]).unwrap().amount, 1000);
        assert_eq!(l.client.order(&l.orders[1]).unwrap().amount, 1000);
        assert_eq!(l.client.order(&l.orders[2]).unwrap().amount, 2000);
    };

    assert_eq!(swap(TradeDirection::Sell, 1000, 1281), Some(709));
    untouched();
    assert_eq!(swap(TradeDirection::Buy, 999, 1280), Some(709));
    untouched();

    let elsewhere = actor(&e);
    token::Client::new(&e, &usd).transfer(&trader, &elsewhere, &1);
    let before = balances(&e, &accounts, &assets);
    // the token's own balance error (`BalanceError` of the Stellar Asset Contract)
    assert_eq!(swap(TradeDirection::Sell, 1000, 1280), Some(10));
    assert_eq!(balances(&e, &accounts, &assets), before);
    assert_eq!(l.client.order(&l.orders[0]).unwrap().amount, 1000);
}
