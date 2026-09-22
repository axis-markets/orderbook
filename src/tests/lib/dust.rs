//! `Sell` rounding leftovers. Selling into an order priced above one unit of the taker's asset per
//! unit of the maker's asset usually leaves part of the input unspendable: it cannot buy a single
//! whole unit. A `FillOrKill` trade or a `Sell` swap counts such a leftover as executed and the
//! trader keeps it; a real liquidity shortage still fails.
use super::setup::{
    actor, assert_no_custody, balance, code, fake_asset, fund, register_axis, setup_test,
    store_order, trade, try_trade,
};
use crate::order::{OrderKind, TradeDirection};
use crate::trade::TradeStep;
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::{Address, Env, Vec};

/// 1.5 USD per EUR
const ONE_AND_HALF: i128 = 15 * PRECISION / 10;
/// Taker limit accepting the 1.5 USD/EUR order: at least 0.6 EUR per USD
const LIMIT_0_6: i128 = 6 * PRECISION / 10;
/// A unit of the maker's asset costing 20,000 units of the taker's asset
const HIGH_UNIT_PRICE: i128 = 20_000 * PRECISION;

/// Contract with a maker selling `amount` EUR for USD at `price` and a taker holding USD.
/// Returns (client, contract address, maker, taker, order id)
fn eur_seller(
    e: &Env,
    usd: &Address,
    eur: &Address,
    amount: i128,
    price: i128,
) -> (AxisClient<'static>, Address, Address, Address, u128) {
    let axis = register_axis(e);
    let client = AxisClient::new(e, &axis);
    let maker = actor(e);
    let taker = actor(e);
    fund(e, eur, &axis, &maker, amount);
    fund(e, usd, &axis, &taker, 10_000_000);
    let id = store_order(&client, &maker, amount, eur, usd, price);
    (client, axis, maker, taker, id)
}

fn single_hop(e: &Env, asset: &Address, order: u128) -> Vec<TradeStep> {
    Vec::from_array(
        e,
        [TradeStep {
            asset: asset.clone(),
            orders: Vec::from_array(e, [order]),
        }],
    )
}

#[test]
fn test_fill_or_kill_accepts_an_unspendable_leftover() {
    let (e, _, _, usd, eur) = setup_test();
    let (client, axis, maker, taker, id) = eur_seller(&e, &usd, &eur, 1000, ONE_AND_HALF);

    // 1000 USD buy 666 EUR for 999 USD; the last 1 USD cannot buy a whole EUR
    let (sold, bought, created) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::FillOrKill,
        &taker,
        1000,
        &usd,
        &eur,
        LIMIT_0_6,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought, created), (999, 666, None));
    assert_eq!(balance(&e, &usd, &taker), 10_000_000 - 999);
    assert_eq!(balance(&e, &eur, &taker), 666);
    assert_eq!(balance(&e, &usd, &maker), 999);
    assert_eq!(client.order(&id).unwrap().amount, 334);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

#[test]
fn test_fill_or_kill_at_a_high_unit_price() {
    let (e, _, _, usd, eur) = setup_test();
    let (client, _, _, taker, id) = eur_seller(&e, &usd, &eur, 100, HIGH_UNIT_PRICE);

    // 1,234,567 USD buy 61 EUR for 1,220,000; the remaining 14,567 cannot buy another EUR
    let (sold, bought, _) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::FillOrKill,
        &taker,
        1_234_567,
        &usd,
        &eur,
        PRECISION / 20_000,
        &Vec::from_array(&e, [id]),
    );
    assert_eq!((sold, bought), (1_220_000, 61));
    assert_eq!(balance(&e, &usd, &taker), 10_000_000 - 1_220_000);
    assert_eq!(balance(&e, &eur, &taker), 61);
}

#[test]
fn test_fill_or_kill_still_fails_on_missing_liquidity() {
    let (e, _, _, usd, eur) = setup_test();
    // only 500 EUR on offer: 1000 USD would need 666
    let (client, _, maker, taker, id) = eur_seller(&e, &usd, &eur, 500, ONE_AND_HALF);

    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::FillOrKill,
            &taker,
            1000,
            &usd,
            &eur,
            LIMIT_0_6,
            &Vec::from_array(&e, [id]),
        ),
        Some(709)
    );
    assert_eq!(balance(&e, &usd, &taker), 10_000_000);
    assert_eq!(balance(&e, &eur, &maker), 500);
    assert_eq!(client.order(&id).unwrap().amount, 500);
}

#[test]
fn test_swap_sell_single_hop_leftover_stays_with_the_trader() {
    let (e, _, _, usd, eur) = setup_test();
    let (client, axis, maker, trader, id) = eur_seller(&e, &usd, &eur, 1000, ONE_AND_HALF);

    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &666,
        &single_hop(&e, &eur, id),
        &None,
    );
    assert_eq!((sold, bought), (999, 666));
    assert_eq!(balance(&e, &usd, &trader), 10_000_000 - 999);
    assert_eq!(balance(&e, &eur, &trader), 666);
    assert_eq!(balance(&e, &usd, &maker), 999);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

#[test]
fn test_swap_sell_intermediate_hop_is_exact() {
    // USD -> EUR at 1.2 USD per EUR, then EUR -> GBP at 1.6 EUR per GBP.
    // Selling 1000 USD yields 833 EUR, but the second hop can spend only 832 of them
    // (520 GBP). The first hop is re-planned to buy exactly 832 EUR for 999 USD, so the
    // contract ends up holding no EUR and the trader keeps 1 USD.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let eur_maker = actor(&e);
    let gbp_maker = actor(&e);
    fund(&e, &eur, &axis, &eur_maker, 833);
    fund(&e, &gbp, &axis, &gbp_maker, 757);
    fund(&e, &usd, &axis, &trader, 1000);
    let eur_order = store_order(&client, &eur_maker, 833, &eur, &usd, 12 * PRECISION / 10);
    let gbp_order = store_order(&client, &gbp_maker, 757, &gbp, &eur, 16 * PRECISION / 10);
    let path = Vec::from_array(
        &e,
        [
            TradeStep {
                asset: eur.clone(),
                orders: Vec::from_array(&e, [eur_order]),
            },
            TradeStep {
                asset: gbp.clone(),
                orders: Vec::from_array(&e, [gbp_order]),
            },
        ],
    );

    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1000,
        &520,
        &path,
        &None,
    );
    assert_eq!((sold, bought), (999, 520));
    assert_eq!(balance(&e, &usd, &trader), 1);
    assert_eq!(balance(&e, &gbp, &trader), 520);
    assert_eq!(balance(&e, &eur, &trader), 0);
    assert_eq!(balance(&e, &usd, &eur_maker), 999);
    assert_eq!(balance(&e, &eur, &eur_maker), 1);
    assert_eq!(balance(&e, &eur, &gbp_maker), 832);
    assert_eq!(balance(&e, &gbp, &gbp_maker), 237);
    assert_eq!(client.order(&eur_order).unwrap().amount, 1);
    assert_eq!(client.order(&gbp_order).unwrap().amount, 237);
    assert_no_custody(&e, &axis, &[&usd, &eur, &gbp]);
}

#[test]
fn test_swap_sell_into_a_high_unit_price_asset() {
    let (e, _, _, usd, eur) = setup_test();
    let (client, axis, _, trader, id) = eur_seller(&e, &usd, &eur, 100, HIGH_UNIT_PRICE);

    let (sold, bought) = client.swap(
        &TradeDirection::Sell,
        &trader,
        &usd,
        &1_234_567,
        &61,
        &single_hop(&e, &eur, id),
        &None,
    );
    assert_eq!((sold, bought), (1_220_000, 61));
    assert_eq!(balance(&e, &usd, &trader), 10_000_000 - 1_220_000);
    assert_no_custody(&e, &axis, &[&usd, &eur]);
}

#[test]
fn test_swap_sell_still_fails_on_missing_liquidity() {
    let (e, _, _, usd, eur) = setup_test();
    // only 500 EUR on offer: the 250 USD left over could buy another 166 EUR
    let (client, _, _, trader, id) = eur_seller(&e, &usd, &eur, 500, ONE_AND_HALF);

    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &trader,
            &usd,
            &1000,
            &1,
            &single_hop(&e, &eur, id),
            &None,
        )),
        Some(709)
    );
    assert_eq!(balance(&e, &usd, &trader), 10_000_000);
}
