//! Resource measurements and restrictions.
//!
//! Every test here builds a book, performs ONE measured contract call last, and reads the
//! host's invocation metering through `env.cost_estimate().resources()`.
//!
//! ```sh
//! cargo test resources -- --nocapture 2>/dev/null | grep 'RES|'
//! ```
//!
//! Caveats: the contract runs natively, so instruction counts are a lower bound
//! (no wasm instantiation); `Address::generate` yields contract addresses, so classic trustline
//! effects (disk reads, missing trustlines) are invisible here.

extern crate std;

use super::setup::{
    actor, advance, fake_asset, fund, list_asset, mainnet_ttls, no_approvals, no_orders,
    order_update, register_axis, removal, set_price, setup_test, store_order, trade,
    MARKET_LISTING_FEE, MIN_PERSISTENT_TTL, UNIT_PRICE,
};
use crate::order::{Order, OrderKind, TradeDirection};
use crate::trade::TradeStep;
use crate::{orderbook::PRECISION, AxisClient};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Env, Vec};
use test_case::test_case;

// ---- current mainnet limits (Stellar mainnet, protocol 23+, after SLP-0004/0005) ----------
pub const TX_INSTRUCTIONS: i64 = 400_000_000;
pub const TX_FOOTPRINT_ENTRIES: u32 = 400;
pub const TX_WRITE_ENTRIES: u32 = 200;
pub const TX_WRITE_BYTES: u32 = 132_096;
pub const TX_EVENT_BYTES: u32 = 16_384;

/// Largest number of maker orders one `Fill` trade can cross under the event-size cap with the
/// current event layout. Derived from the `fill_n` probes (docs/03); pinned by the
/// `fill MAX_FILLS` case below.
pub const MAX_FILLS: u32 = 20;

const ORDER_AMOUNT: i128 = 1_000_000;

/// Env with mock auth and mainnet TTLs; metering on, the SDK's stale limit enforcement off.
/// Gated tests assert the current caps through `Usage::within_tx_limits`.
fn gate_env() -> (Env, Address, Address, Address, Address) {
    let (e, trader, issuer, usd, eur) = setup_test();
    mainnet_ttls(&e);
    e.cost_estimate().disable_resource_limits();
    // The test host's own metering budget is far below the network's 400M; lift it so large
    // probes are limited by our assertions, not by the harness.
    e.cost_estimate().budget().reset_unlimited();
    (e, trader, issuer, usd, eur)
}

/// Same environment for probes that may exceed a per-transaction cap on purpose.
fn probe_env() -> (Env, Address, Address, Address, Address) {
    gate_env()
}

#[derive(Debug, Clone, Copy)]
struct Usage {
    reads: u32,
    writes: u32,
    write_bytes: u32,
    event_bytes: u32,
    instructions: i64,
    /// Bytes of persistent entries created (or restored) by the call, recovered from
    /// `persistent_rent_ledger_bytes / MIN_PERSISTENT_TTL`.
    rent_entry_bytes: i64,
    rent_bumps: u32,
}

impl Usage {
    fn within_tx_limits(&self) -> bool {
        self.reads + self.writes <= TX_FOOTPRINT_ENTRIES
            && self.writes <= TX_WRITE_ENTRIES
            && self.write_bytes <= TX_WRITE_BYTES
            && self.event_bytes <= TX_EVENT_BYTES
            && self.instructions <= TX_INSTRUCTIONS
    }
}

/// Read the metering of the last top-level invocation and print one table row.
fn measure(e: &Env, name: &str) -> Usage {
    let r = e.cost_estimate().resources();
    let u = Usage {
        reads: r.memory_read_entries,
        writes: r.write_entries,
        write_bytes: r.write_bytes,
        event_bytes: r.contract_events_size_bytes,
        instructions: r.instructions,
        rent_entry_bytes: r.persistent_rent_ledger_bytes / MIN_PERSISTENT_TTL as i64,
        rent_bumps: r.persistent_entry_rent_bumps,
    };
    std::println!(
        "RES| {} | {} | {} | {} | {} | {} | {} | {} |",
        name,
        u.reads,
        u.writes,
        u.write_bytes,
        u.event_bytes,
        u.instructions,
        u.rent_entry_bytes,
        u.rent_bumps
    );
    u
}

/// Gate: the row must fit the transaction and stay under a recorded ceiling
/// (first measurement rounded up, ~25% headroom).
fn gate(u: &Usage, max_writes: u32, max_write_bytes: u32, max_event_bytes: u32) {
    assert!(u.within_tx_limits(), "exceeds per-tx limits: {:?}", u);
    assert!(
        u.writes <= max_writes,
        "writes {} > {}",
        u.writes,
        max_writes
    );
    assert!(
        u.write_bytes <= max_write_bytes,
        "write bytes {} > {}",
        u.write_bytes,
        max_write_bytes
    );
    assert!(
        u.event_bytes <= max_event_bytes,
        "event bytes {} > {}",
        u.event_bytes,
        max_event_bytes
    );
}

/// Register the contract and create `n` sell orders of `ORDER_AMOUNT` `sell` for `buy` at price 1,
/// each backed by the maker's balance and allowance.
/// With `distinct_makers` every order has its own owner; otherwise one maker owns them all.
/// Returns (client, contract address, maker addresses, order ids).
fn seed_book<'a>(
    e: &'a Env,
    sell: &Address,
    buy: &Address,
    n: u32,
    distinct_makers: bool,
) -> (AxisClient<'a>, Address, std::vec::Vec<Address>, Vec<u128>) {
    let contract = register_axis(e);
    let client = AxisClient::new(e, &contract);
    let mut makers = std::vec::Vec::new();
    let mut ids = Vec::new(e);
    let shared = actor(e);
    for _ in 0..n {
        let maker = if distinct_makers {
            actor(e)
        } else {
            shared.clone()
        };
        fund(e, sell, &contract, &maker, ORDER_AMOUNT);
        let id = store_order(&client, &maker, ORDER_AMOUNT, sell, buy, PRECISION);
        ids.push_back(id);
        makers.push(maker);
    }
    (client, contract, makers, ids)
}

/// Taker sells `n * ORDER_AMOUNT + extra` of `sell` for `buy` against `ids`.
#[allow(clippy::too_many_arguments)]
fn take(
    e: &Env,
    client: &AxisClient,
    taker: &Address,
    sell: &Address,
    buy: &Address,
    kind: OrderKind,
    n: u32,
    extra: i128,
    ids: &Vec<u128>,
) -> (i128, i128, Option<u128>) {
    let amount = n as i128 * ORDER_AMOUNT + extra;
    fund(e, sell, &client.address, taker, amount);
    trade(
        client,
        TradeDirection::Sell,
        kind,
        taker,
        amount,
        sell,
        buy,
        PRECISION,
        ids,
    )
}

#[test]
fn gate_existing_order_sell_limit() {
    let (e, trader, _, usd, eur) = gate_env();
    // One order already stored, so the market exists and the price is
    // cached: the only persistent entry this call creates is the new order.
    let (client, contract, _, _) = seed_book(&e, &usd, &eur, 1, true);
    fund(&e, &usd, &contract, &trader, ORDER_AMOUNT);
    let id = store_order(&client, &trader, ORDER_AMOUNT, &usd, &eur, PRECISION);
    let u = measure(&e, "trade Sell Limit, store only (new order)");
    // Size of the stored order value (ScVal XDR) vs the full ledger entry the rent is charged on.
    let value_xdr = e.as_contract(&contract, || {
        let o: Order = e.storage().persistent().get(&id).unwrap();
        o.to_xdr(&e).len()
    });
    std::println!(
        "RES| order entry: value XDR {} B, ledger entry (rent) {} B |",
        value_xdr,
        u.rent_entry_bytes
    );
    assert!(
        u.rent_entry_bytes > value_xdr as i64,
        "rent covers the full ledger entry"
    );
    gate(&u, 3, 600, 320);
}

#[test]
fn gate_open_order_buy_limit() {
    let (e, trader, _, usd, eur) = gate_env();
    // One USD-selling order already exists, so the market exists and the USD price is cached.
    let (client, contract, _, _) = seed_book(&e, &usd, &eur, 1, true);
    fund(&e, &usd, &contract, &trader, ORDER_AMOUNT);
    trade(
        &client,
        TradeDirection::Buy,
        OrderKind::Limit,
        &trader,
        ORDER_AMOUNT,
        &usd,
        &eur,
        PRECISION,
        &no_orders(&e),
    );
    let u = measure(&e, "trade Buy Limit, create only (new order)");
    gate(&u, 3, 600, 320);
}

#[test]
fn measure_subsidize_new_market() {
    let (e, trader, issuer, usd, _) = gate_env();
    // Fresh pair with both assets listed: `subsidize` opens the market (oracle listing probes,
    // token decimals, `track` with XRF burn for the listing fee, then for the subsidy)
    let gbp = fake_asset(&e, &issuer);
    list_asset(&e, &gbp, UNIT_PRICE);
    let contract = register_axis(&e);
    let client = AxisClient::new(&e, &contract);
    // the listing fee opens the market, the other half of the amount is a subsidy on top
    client.subsidize(&trader, &usd, &gbp, &(2 * MARKET_LISTING_FEE));
    let u = measure(&e, "subsidize, new market (two tracks, two price fetches)");
    gate(&u, 8, 3_400, 560);
}

#[test]
fn measure_requote() {
    let (e, _, _, usd, eur) = gate_env();
    let (client, _, _, _) = seed_book(&e, &usd, &eur, 1, true);
    // A keeper re-checks the market and pulls fresh quotes for both assets into the cache
    advance(&e, 3600);
    set_price(&e, &usd, UNIT_PRICE);
    set_price(&e, &eur, UNIT_PRICE);
    client.requote(&usd, &eur);
    let u = measure(&e, "requote (two price fetches)");
    gate(&u, 4, 2_200, 0);
}

#[test]
fn measure_subsidize() {
    let (e, trader, _, usd, eur) = gate_env();
    let (client, _, _, _) = seed_book(&e, &usd, &eur, 1, true);
    client.subsidize(&trader, &usd, &eur, &MARKET_LISTING_FEE);
    let u = measure(&e, "subsidize, existing market (two price fetches)");
    gate(&u, 6, 2_600, 240);
}

// ---- update: removal / modification --------------------------------------------------------

#[test_case(1, true; "remove 1")]
#[test_case(8, true; "remove 8")]
#[test_case(30, true; "remove 30")]
#[test_case(64, false; "remove 64 (probe)")]
fn measure_remove_n(n: u32, gated: bool) {
    let (e, _, _, usd, eur) = if gated { gate_env() } else { probe_env() };
    let (client, _, makers, ids) = seed_book(&e, &eur, &usd, n, false);
    let mut removals = Vec::new(&e);
    for id in ids.iter() {
        removals.push_back(removal(id));
    }
    let removed = client.update(&makers[0], &removals, &no_approvals(&e));
    assert_eq!(removed.len(), n);
    let u = measure(&e, &std::format!("update: remove {} orders (one owner)", n));
    assert_eq!(u.rent_entry_bytes, 0, "a removal creates nothing");
    if gated {
        assert!(u.within_tx_limits(), "{:?}", u);
    }
}

#[test_case(1; "update 1")]
#[test_case(8; "update 8")]
fn gate_update_n(n: u32) {
    let (e, _, _, usd, eur) = gate_env();
    let (client, _, makers, ids) = seed_book(&e, &eur, &usd, n, false);
    let mut updates = Vec::new(&e);
    for id in ids.iter() {
        updates.push_back(order_update(id, ORDER_AMOUNT / 2, 2 * PRECISION));
    }
    let updated = client.update(&makers[0], &updates, &no_approvals(&e));
    assert_eq!(updated.len(), n);
    let u = measure(&e, &std::format!("update {} orders (one owner)", n));
    assert!(u.within_tx_limits(), "{:?}", u);
}

// ---- fill ---------------------------------------------------------------------------------

#[test_case(1, true; "fill 1")]
#[test_case(8, true; "fill 8")]
#[test_case(MAX_FILLS, true; "fill MAX_FILLS")]
#[test_case(MAX_FILLS + 1, false; "fill MAX_FILLS+1 (probe, expected over the event cap)")]
#[test_case(24, false; "fill 24 (probe)")]
#[test_case(32, false; "fill 32 (probe)")]
fn measure_fill_n_distinct_makers(n: u32, gated: bool) {
    let (e, taker, _, usd, eur) = if gated { gate_env() } else { probe_env() };
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, n, true);
    let (sold, bought, id) = take(&e, &client, &taker, &usd, &eur, OrderKind::Fill, n, 0, &ids);
    assert_eq!(sold, n as i128 * ORDER_AMOUNT);
    assert_eq!(bought, sold);
    assert_eq!(id, None);
    let u = measure(
        &e,
        &std::format!("trade Fill crossing {} orders, {} makers", n, n),
    );
    // A pure fill creates no order entry; the only rent is first-time SAC balance entries of
    // the counterparties (each maker receives USD for the first time here).
    if gated {
        assert!(u.within_tx_limits(), "{:?}", u);
    } else if n == MAX_FILLS + 1 {
        assert!(
            u.event_bytes > TX_EVENT_BYTES,
            "MAX_FILLS is stale: {:?}",
            u
        );
    }
}

#[test]
fn gate_fill_8_single_maker() {
    let (e, taker, _, usd, eur) = gate_env();
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, 8, false);
    take(&e, &client, &taker, &usd, &eur, OrderKind::Fill, 8, 0, &ids);
    let u = measure(&e, "trade Fill crossing 8 orders, 1 maker");
    assert!(u.within_tx_limits(), "{:?}", u);
}

#[test]
fn gate_fill_8_and_create_order() {
    let (e, taker, _, usd, eur) = gate_env();
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, 8, true);
    let (_, _, id) = take(
        &e,
        &client,
        &taker,
        &usd,
        &eur,
        OrderKind::Limit,
        8,
        ORDER_AMOUNT,
        &ids,
    );
    assert!(id.is_some(), "remainder must be stored");
    let u = measure(
        &e,
        "trade Limit crossing 8 orders + remainder order created",
    );
    assert!(u.within_tx_limits(), "{:?}", u);
}

#[test]
fn gate_fill_8_unbacked_makers() {
    // Every maker revoked their allowance: the taker pays the pre-reads and the removals
    let (e, taker, _, usd, eur) = gate_env();
    let (client, contract, makers, ids) = seed_book(&e, &eur, &usd, 8, true);
    for maker in makers.iter() {
        super::setup::approve(&e, &eur, &contract, maker, 0);
    }
    let (sold, _, _) = take(&e, &client, &taker, &usd, &eur, OrderKind::Fill, 8, 0, &ids);
    assert_eq!(sold, 0);
    let u = measure(&e, "trade Fill over 8 unbacked orders (all removed)");
    assert!(u.within_tx_limits(), "{:?}", u);
}

// ---- swap ---------------------------------------------------------------------------------

/// Chain of markets: asset[i] is sold for asset[i+1] by `per_hop` makers at price 1.
fn seed_chain<'a>(
    e: &'a Env,
    assets: &[Address],
    per_hop: u32,
) -> (AxisClient<'a>, Address, Vec<TradeStep>) {
    let contract = register_axis(e);
    let client = AxisClient::new(e, &contract);
    let mut path = Vec::new(e);
    for hop in 0..assets.len() - 1 {
        let (sell, buy) = (&assets[hop], &assets[hop + 1]);
        let mut ids = Vec::new(e);
        for _ in 0..per_hop {
            let maker = actor(e);
            fund(e, buy, &contract, &maker, ORDER_AMOUNT);
            let id = store_order(&client, &maker, ORDER_AMOUNT, buy, sell, PRECISION);
            ids.push_back(id);
        }
        path.push_back(TradeStep {
            asset: buy.clone(),
            orders: ids,
        });
    }
    (client, contract, path)
}

#[test_case(2, 1, TradeDirection::Sell; "swap 2 hops x 1 order, Sell")]
#[test_case(2, 1, TradeDirection::Buy; "swap 2 hops x 1 order, Buy")]
#[test_case(2, 4, TradeDirection::Sell; "swap 2 hops x 4 orders, Sell")]
#[test_case(3, 1, TradeDirection::Sell; "swap 3 hops x 1 order, Sell")]
fn gate_swap(hops: u32, per_hop: u32, direction: TradeDirection) {
    let (e, trader, issuer, usd, eur) = gate_env();
    let mut assets = std::vec![usd.clone(), eur.clone()];
    for _ in 2..=hops {
        let asset = fake_asset(&e, &issuer);
        list_asset(&e, &asset, UNIT_PRICE);
        assets.push(asset);
    }
    let (client, contract, path) = seed_chain(&e, &assets, per_hop);
    let amount = per_hop as i128 * ORDER_AMOUNT;
    fund(&e, &usd, &contract, &trader, amount);
    let (sold, bought) = client.swap(&direction, &trader, &usd, &amount, &amount, &path, &None);
    assert_eq!(sold, amount);
    assert_eq!(bought, amount);
    let u = measure(
        &e,
        &std::format!("swap {:?} {} hops x {} orders", direction, hops, per_hop),
    );
    assert!(u.within_tx_limits(), "{:?}", u);
}

// ---- crossfill (crank) -------------------------------------------------------------------

/// Taker order sells `n * ORDER_AMOUNT` USD for EUR at 1; `n` makers sell EUR for USD at 1.
fn seed_cross<'a>(
    e: &'a Env,
    usd: &Address,
    eur: &Address,
    n: u32,
) -> (AxisClient<'a>, u128, Vec<u128>) {
    let (client, contract, _, maker_ids) = seed_book(e, eur, usd, n, true);
    let owner = actor(e);
    let amount = n as i128 * ORDER_AMOUNT;
    fund(e, usd, &contract, &owner, amount);
    let taker_id = store_order(&client, &owner, amount, usd, eur, PRECISION);
    (client, taker_id, maker_ids)
}

#[test]
fn gate_crossfill_8() {
    let (e, _, _, usd, eur) = gate_env();
    let (client, taker_id, maker_ids) = seed_cross(&e, &usd, &eur, 8);
    let cranker = actor(&e);
    let (sold, bought, profit) = client.crossfill(&cranker, &taker_id, &maker_ids);
    assert_eq!(sold, 8 * ORDER_AMOUNT);
    assert_eq!(bought, sold);
    assert_eq!(profit, 0);
    let u = measure(
        &e,
        "crossfill crossing 8 maker orders (cranker holds nothing)",
    );
    assert!(u.within_tx_limits(), "{:?}", u);
}

/// Settlement no longer depends on address ordering: both legs are `transfer_from` between the
/// taker order's owner and the makers, so a cranker sorting before the contract (every
/// G-account cranker in production) needs no float.
#[test]
fn crossfill_cranker_sorting_before_contract_needs_no_float() {
    let (e, _, _, usd, eur) = probe_env();
    let cranker = actor(&e); // generated BEFORE the contract is registered
    let (client, taker_id, maker_ids) = seed_cross(&e, &usd, &eur, 1);
    let (sold, bought, profit) = client.crossfill(&cranker, &taker_id, &maker_ids);
    assert_eq!((sold, bought, profit), (ORDER_AMOUNT, ORDER_AMOUNT, 0));
}

// ---- keepalive / views --------------------------------------------------------------------

#[test]
fn gate_keepalive() {
    let (e, _, _, usd, eur) = gate_env();
    let (client, _, _, _) = seed_book(&e, &eur, &usd, 1, true);
    client.keepalive();
    let u = measure(&e, "keepalive");
    assert!(u.within_tx_limits(), "{:?}", u);
    assert_eq!(u.event_bytes, 0);
}

#[test]
fn measure_views() {
    let (e, _, _, usd, eur) = gate_env();
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, 1, true);
    client.order(&ids.get(0).unwrap());
    let u = measure(&e, "order (view)");
    assert_eq!(u.writes, 0);
}
