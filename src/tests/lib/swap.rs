use super::setup::{fake_asset, setup_test};
use crate::order::{OrderKind, TradeDirection};
use crate::{orderbook::PRECISION, trade::TradeStep, Axis, AxisClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};

// Cross rates:
//   EUR/USD = 1.2  (1 EUR costs 1.2 USD)
//   GBP/USD = 1.32 (1 GBP costs 1.32 USD)  =>  GBP/EUR = 1.32 / 1.2 = 1.1
const EUR_USD: i128 = 12 * PRECISION / 10; // 1.2 USD per EUR
const GBP_EUR: i128 = 11 * PRECISION / 10; // 1.1 EUR per GBP

/// Set up the USD/EUR and EUR/GBP markets used by the two-hop tests.
/// `maker1` sells `eur_amount` EUR for USD at 1.2; `maker2` sells `gbp_amount` GBP for EUR at 1.1.
/// Returns (client, contract_address, gbp, maker1, maker2, order1, order2) plus token clients.
fn two_market_setup<'a>(
    e: &Env,
    usd: &Address,
    eur: &Address,
    gbp: &Address,
    eur_amount: i128,
    gbp_amount: i128,
) -> (AxisClient<'a>, Address, Address, Address, u64, u64) {
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(e, &contract_address);

    let maker1 = Address::generate(e);
    let maker2 = Address::generate(e);

    StellarAssetClient::new(e, eur).mint(&maker1, &eur_amount);
    StellarAssetClient::new(e, gbp).mint(&maker2, &gbp_amount);

    let (_, _, order1) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker1,
        &eur_amount,
        eur,
        usd,
        &EUR_USD,
        &Vec::new(e),
    );
    let (_, _, order2) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker2,
        &gbp_amount,
        gbp,
        eur,
        &GBP_EUR,
        &Vec::new(e),
    );

    (client, contract_address, maker1, maker2, order1, order2)
}

/// Read the internal trade-id counter directly.
fn last_trade_id(e: &Env, contract: &Address) -> u64 {
    e.as_contract(contract, || crate::trade::get_last_trade_id(e))
}

fn two_hop_path(e: &Env, eur: &Address, gbp: &Address, order1: u64, order2: u64) -> Vec<TradeStep> {
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

    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);
    let gbp_client = StellarAssetClient::new(&e, &gbp);
    usd_client.mint(&trader, &1000);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    let (sold, bought) = client.swap(&TradeDirection::Sell, &trader, &usd, &1000, &757, &path);

    assert_eq!(sold, 1000);
    assert_eq!(bought, 757);

    // trader: spent all USD, received GBP, no EUR residue
    assert_eq!(usd_client.balance(&trader), 0);
    assert_eq!(gbp_client.balance(&trader), 757);
    assert_eq!(eur_client.balance(&trader), 0);

    // makers received their proceeds; both orders fully consumed
    assert_eq!(usd_client.balance(&maker1), 1000);
    assert_eq!(eur_client.balance(&maker1), 0);
    assert_eq!(eur_client.balance(&maker2), 833);
    assert_eq!(gbp_client.balance(&maker2), 0);
    assert!(client.order(&order1).is_none());
    assert!(client.order(&order2).is_none());

    // contract nets to zero across every asset
    assert_eq!(usd_client.balance(&contract_address), 0);
    assert_eq!(eur_client.balance(&contract_address), 0);
    assert_eq!(gbp_client.balance(&contract_address), 0);

    // two legs settled => sequential trade ids 1 and 2 were assigned
    assert_eq!(last_trade_id(&e, &contract_address), 2);
}

#[test]
fn test_swap_buy_two_hops() {
    // Buy exactly 100 GBP via EUR using USD, spending at most 200 USD.
    //   100 GBP * 1.1 = 110 EUR, 110 EUR * 1.2 = 132 USD
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, maker1, maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 110, 100);

    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);
    let gbp_client = StellarAssetClient::new(&e, &gbp);
    usd_client.mint(&trader, &200);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    let (sold, bought) = client.swap(&TradeDirection::Buy, &trader, &usd, &200, &100, &path);

    assert_eq!(bought, 100);
    assert_eq!(sold, 132);

    // trader keeps the unused 68 USD surplus
    assert_eq!(usd_client.balance(&trader), 68);
    assert_eq!(gbp_client.balance(&trader), 100);
    assert_eq!(eur_client.balance(&trader), 0);

    assert_eq!(usd_client.balance(&maker1), 132);
    assert_eq!(eur_client.balance(&maker2), 110);
    assert!(client.order(&order1).is_none());
    assert!(client.order(&order2).is_none());

    assert_eq!(usd_client.balance(&contract_address), 0);
    assert_eq!(eur_client.balance(&contract_address), 0);
    assert_eq!(gbp_client.balance(&contract_address), 0);

    assert_eq!(last_trade_id(&e, &contract_address), 2);
}

#[test]
fn test_swap_sell_slippage_kill() {
    // Same liquidity as the happy path, but demand more GBP than the route yields.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, _maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 833, 757);

    let usd_client = StellarAssetClient::new(&e, &usd);
    let gbp_client = StellarAssetClient::new(&e, &gbp);
    usd_client.mint(&trader, &1000);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    // route yields 757 GBP but we require at least 758 -> killed
    let (sold, bought) = client.swap(&TradeDirection::Sell, &trader, &usd, &1000, &758, &path);

    assert_eq!((sold, bought), (0, 0));

    // nothing moved, no trade id consumed, no order touched
    assert_eq!(usd_client.balance(&trader), 1000);
    assert_eq!(gbp_client.balance(&trader), 0);
    assert_eq!(usd_client.balance(&contract_address), 0);
    assert_eq!(last_trade_id(&e, &contract_address), 0);
    assert_eq!(client.order(&order1).unwrap().amount, 833);
    assert_eq!(client.order(&order2).unwrap().amount, 757);
}

#[test]
fn test_swap_buy_slippage_kill() {
    // Buy 100 GBP would cost 132 USD, but cap spending at 131 -> killed.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, _maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 110, 100);

    let usd_client = StellarAssetClient::new(&e, &usd);
    let gbp_client = StellarAssetClient::new(&e, &gbp);
    usd_client.mint(&trader, &200);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    let (sold, bought) = client.swap(&TradeDirection::Buy, &trader, &usd, &131, &100, &path);

    assert_eq!((sold, bought), (0, 0));
    assert_eq!(usd_client.balance(&trader), 200);
    assert_eq!(gbp_client.balance(&trader), 0);
    assert_eq!(last_trade_id(&e, &contract_address), 0);
    assert_eq!(client.order(&order1).unwrap().amount, 110);
    assert_eq!(client.order(&order2).unwrap().amount, 100);
}

#[test]
fn test_swap_sell_insufficient_liquidity_kill() {
    // Second hop has only 100 GBP of liquidity, far short of the 757 the route needs,
    // so the first hop's full EUR output cannot be sold onward -> killed.
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);

    let (client, contract_address, _maker1, _maker2, order1, order2) =
        two_market_setup(&e, &usd, &eur, &gbp, 833, 100);

    let usd_client = StellarAssetClient::new(&e, &usd);
    let gbp_client = StellarAssetClient::new(&e, &gbp);
    usd_client.mint(&trader, &1000);

    let path = two_hop_path(&e, &eur, &gbp, order1, order2);
    let (sold, bought) = client.swap(&TradeDirection::Sell, &trader, &usd, &1000, &1, &path);

    assert_eq!((sold, bought), (0, 0));
    assert_eq!(usd_client.balance(&trader), 1000);
    assert_eq!(gbp_client.balance(&trader), 0);
    assert_eq!(usd_client.balance(&contract_address), 0);
    assert_eq!(last_trade_id(&e, &contract_address), 0);
    assert_eq!(client.order(&order1).unwrap().amount, 833);
    assert_eq!(client.order(&order2).unwrap().amount, 100);
}

#[test]
fn test_swap_single_hop_like_fill_or_kill() {
    // A one-step path behaves like a FillOrKill trade in a single market.
    let (e, trader, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let maker = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);
    usd_client.mint(&trader, &1000);
    eur_client.mint(&maker, &833);

    let (_, _, order1) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &833,
        &eur,
        &usd,
        &EUR_USD,
        &Vec::new(&e),
    );

    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: eur.clone(),
            orders: Vec::from_array(&e, [order1]),
        }],
    );
    let (sold, bought) = client.swap(&TradeDirection::Sell, &trader, &usd, &1000, &833, &path);

    assert_eq!(sold, 1000);
    assert_eq!(bought, 833);
    assert_eq!(usd_client.balance(&trader), 0);
    assert_eq!(eur_client.balance(&trader), 833);
    assert_eq!(usd_client.balance(&maker), 1000);
    assert!(client.order(&order1).is_none());
}

#[test]
#[should_panic(expected = "#705")]
fn test_swap_wrong_asset_in_step_panics() {
    // The step claims to buy GBP but points at a USD/EUR order -> InvalidMatch (705).
    let (e, trader, issuer, usd, eur) = setup_test();
    let gbp = fake_asset(&e, &issuer);
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let maker = Address::generate(&e);
    StellarAssetClient::new(&e, &usd).mint(&trader, &1000);
    StellarAssetClient::new(&e, &eur).mint(&maker, &1000);

    let (_, _, order1) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &833,
        &eur,
        &usd,
        &EUR_USD,
        &Vec::new(&e),
    );

    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: gbp.clone(),
            orders: Vec::from_array(&e, [order1]),
        }],
    );
    client.swap(&TradeDirection::Sell, &trader, &usd, &1000, &1, &path);
}

#[test]
fn test_fillorkill_consumes_no_trade_id() {
    // An empty FillOrKill must not consume a trade id (or emit any event), even though
    // it partially matched while computing the fill.
    let (e, maker, _, usd, eur) = setup_test();
    let contract_address = e.register(Axis, ());
    let client = AxisClient::new(&e, &contract_address);

    let taker = Address::generate(&e);
    let usd_client = StellarAssetClient::new(&e, &usd);
    let eur_client = StellarAssetClient::new(&e, &eur);
    usd_client.mint(&maker, &10000);
    eur_client.mint(&taker, &10000);

    // maker sells only 300 USD wanting 1.2 EUR per USD
    let (_, _, order_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &maker,
        &300,
        &usd,
        &eur,
        &EUR_USD,
        &Vec::new(&e),
    );

    // taker FillOrKill: sell 1000 EUR for USD, but only 300 USD of liquidity exists -> killed
    let (sold, bought, created) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::FillOrKill,
        &taker,
        &1000,
        &eur,
        &usd,
        &(PRECISION / 2),
        &Vec::from_array(&e, [order_id]),
    );
    assert_eq!((sold, bought, created), (0, 0, 0));

    // the partial match never settled: no trade id consumed, balances and order untouched
    assert_eq!(last_trade_id(&e, &contract_address), 0);
    assert_eq!(eur_client.balance(&taker), 10000);
    assert_eq!(usd_client.balance(&taker), 0);
    assert_eq!(client.order(&order_id).unwrap().amount, 300);
}
