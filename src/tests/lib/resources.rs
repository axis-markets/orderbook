//! Resource measurements and restrictions.
//!
//! Every test here builds a book, performs ONE measured contract call last, and reads the
//! host's invocation metering through `env.cost_estimate().resources()`.
//!
//! ```sh
//! cargo test resources -- --nocapture 2>/dev/null | grep 'RES|'
//! ```
//!//! Caveats: the contract runs natively, so instruction counts are a lower bound
//! (no wasm instantiation); `Address::generate` yields contract addresses, so classic trustline
//! effects (disk reads, missing trustlines) are invisible here.

extern crate std;

use super::setup::{fake_asset, setup_test};
use crate::order::{Order, OrderKind, TradeDirection};
use crate::trade::TradeStep;
use crate::{orderbook::PRECISION, Axis, AxisClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};
use test_case::test_case;

// ---- current mainnet limits (Stellar mainnet, protocol 23+, after SLP-0004/0005) ----------
pub const TX_INSTRUCTIONS: i64 = 400_000_000;
pub const TX_FOOTPRINT_ENTRIES: u32 = 400;
pub const TX_WRITE_ENTRIES: u32 = 200;
pub const TX_WRITE_BYTES: u32 = 132_096;
pub const TX_EVENT_BYTES: u32 = 16_384;
pub const MIN_PERSISTENT_TTL: u32 = 2_073_600;
pub const MAX_ENTRY_TTL: u32 = 3_110_400;

/// Largest number of maker orders one `Fill` trade can cross under the event-size cap with the
/// current event layout. Derived from the `fill_n` probes (docs/03); pinned by the
/// `fill MAX_FILLS` case below.
pub const MAX_FILLS: u32 = 12;

const ORDER_AMOUNT: i128 = 1_000_000;

fn mainnet_ttls(e: &Env) {
    e.ledger().set_min_persistent_entry_ttl(MIN_PERSISTENT_TTL);
    e.ledger().set_max_entry_ttl(MAX_ENTRY_TTL);
}

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

/// Register the contract and rest `n` sell orders of `ORDER_AMOUNT` `sell` for `buy` at price 1.
/// With `distinct_makers` every order has its own owner; otherwise one maker owns them all.
/// Returns (client, contract address, maker addresses, order ids).
fn seed_book<'a>(
    e: &'a Env,
    sell: &Address,
    buy: &Address,
    n: u32,
    distinct_makers: bool,
) -> (AxisClient<'a>, Address, std::vec::Vec<Address>, Vec<u64>) {
    let contract = e.register(Axis, ());
    let client = AxisClient::new(e, &contract);
    let sell_token = StellarAssetClient::new(e, sell);
    let mut makers = std::vec::Vec::new();
    let mut ids = Vec::new(e);
    let shared = Address::generate(e);
    for _ in 0..n {
        let maker = if distinct_makers {
            Address::generate(e)
        } else {
            shared.clone()
        };
        sell_token.mint(&maker, &ORDER_AMOUNT);
        let (_, _, id) = client.trade(
            &TradeDirection::Sell,
            &OrderKind::Limit,
            &maker,
            &ORDER_AMOUNT,
            sell,
            buy,
            &PRECISION,
            &Vec::new(e),
        );
        ids.push_back(id);
        makers.push(maker);
    }
    (client, contract, makers, ids)
}

/// Taker sells `n * ORDER_AMOUNT + extra` of `sell` for `buy` against `ids`.
fn take(
    e: &Env,
    client: &AxisClient,
    taker: &Address,
    sell: &Address,
    buy: &Address,
    kind: OrderKind,
    n: u32,
    extra: i128,
    ids: &Vec<u64>,
) -> (i128, i128, u64) {
    let amount = n as i128 * ORDER_AMOUNT + extra;
    StellarAssetClient::new(e, sell).mint(taker, &amount);
    client.trade(
        &TradeDirection::Sell,
        &kind,
        taker,
        &amount,
        sell,
        buy,
        &PRECISION,
        ids,
    )
}

// ---- rest ---------------------------------------------------------------------------------

#[test]
fn gate_rest_sell_limit() {
    let (e, trader, _, usd, eur) = gate_env();
    // One order already rests in the same direction, so the contract's USD balance entry
    // exists and the only persistent entry this call creates is the new order.
    let (client, contract, _, _) = seed_book(&e, &usd, &eur, 1, true);
    StellarAssetClient::new(&e, &usd).mint(&trader, &ORDER_AMOUNT);
    let (_, _, id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &trader,
        &ORDER_AMOUNT,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );
    let u = measure(&e, "trade Sell Limit, rest only (new order)");
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
    gate(&u, 6, 1_400, 1_000);
}

#[test]
fn gate_rest_buy_limit() {
    let (e, trader, _, usd, eur) = gate_env();
    let (client, _, _, _) = seed_book(&e, &eur, &usd, 0, true);
    StellarAssetClient::new(&e, &usd).mint(&trader, &ORDER_AMOUNT);
    client.trade(
        &TradeDirection::Buy,
        &OrderKind::Limit,
        &trader,
        &ORDER_AMOUNT,
        &usd,
        &eur,
        &PRECISION,
        &Vec::new(&e),
    );
    let u = measure(&e, "trade Buy Limit, rest only (new order)");
    gate(&u, 6, 1_400, 1_000);
}

// ---- cancel -------------------------------------------------------------------------------

#[test_case(1, true; "cancel 1")]
#[test_case(8, true; "cancel 8")]
#[test_case(30, false; "cancel 30 (probe)")]
#[test_case(32, false; "cancel 32 (probe)")]
fn measure_cancel_n(n: u32, gated: bool) {
    let (e, _, _, usd, eur) = if gated { gate_env() } else { probe_env() };
    let (client, _, makers, ids) = seed_book(&e, &eur, &usd, n, false);
    client.cancel(&ids, &makers[0]);
    let u = measure(&e, &std::format!("cancel {} orders (one owner)", n));
    assert_eq!(u.rent_entry_bytes, 0, "cancel creates nothing");
    if gated {
        assert!(u.within_tx_limits(), "{:?}", u);
    }
}

// ---- fill ---------------------------------------------------------------------------------

#[test_case(1, true; "fill 1")]
#[test_case(8, true; "fill 8")]
#[test_case(MAX_FILLS, true; "fill MAX_FILLS")]
#[test_case(MAX_FILLS + 1, false; "fill MAX_FILLS+1 (probe, expected over the event cap)")]
#[test_case(16, false; "fill 16 (probe)")]
#[test_case(32, false; "fill 32 (probe)")]
fn measure_fill_n_distinct_makers(n: u32, gated: bool) {
    let (e, taker, _, usd, eur) = if gated { gate_env() } else { probe_env() };
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, n, true);
    let (sold, bought, id) = take(&e, &client, &taker, &usd, &eur, OrderKind::Fill, n, 0, &ids);
    assert_eq!(sold, n as i128 * ORDER_AMOUNT);
    assert_eq!(bought, sold);
    assert_eq!(id, 0);
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
fn gate_fill_8_and_rest() {
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
    assert!(id > 0, "remainder must rest");
    let u = measure(&e, "trade Limit crossing 8 orders + rest remainder");
    assert!(u.within_tx_limits(), "{:?}", u);
}

#[test]
fn gate_fill_or_kill_unfilled_still_pays() {
    // FillOrKill that cannot be filled returns (0,0,0) instead of failing: the caller pays for
    // the whole matching pass. Recorded here so the cost of the silent no-op is visible.
    let (e, taker, _, usd, eur) = gate_env();
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, 8, true);
    let (sold, _, _) = take(
        &e,
        &client,
        &taker,
        &usd,
        &eur,
        OrderKind::FillOrKill,
        8,
        ORDER_AMOUNT,
        &ids,
    );
    assert_eq!(sold, 0);
    let u = measure(
        &e,
        "trade FillOrKill over 8 orders, unfilled (silent no-op)",
    );
    assert!(u.within_tx_limits(), "{:?}", u);
}

// ---- swap ---------------------------------------------------------------------------------

/// Chain of markets: asset[i] is sold for asset[i+1] by `per_hop` makers at price 1.
fn seed_chain<'a>(
    e: &'a Env,
    assets: &[Address],
    per_hop: u32,
) -> (AxisClient<'a>, Address, Vec<TradeStep>) {
    let contract = e.register(Axis, ());
    let client = AxisClient::new(e, &contract);
    let mut path = Vec::new(e);
    for hop in 0..assets.len() - 1 {
        let (sell, buy) = (&assets[hop], &assets[hop + 1]);
        let mut ids = Vec::new(e);
        for _ in 0..per_hop {
            let maker = Address::generate(e);
            StellarAssetClient::new(e, buy).mint(&maker, &ORDER_AMOUNT);
            let (_, _, id) = client.trade(
                &TradeDirection::Sell,
                &OrderKind::Limit,
                &maker,
                &ORDER_AMOUNT,
                buy,
                sell,
                &PRECISION,
                &Vec::new(e),
            );
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
        assets.push(fake_asset(&e, &issuer));
    }
    let (client, _, path) = seed_chain(&e, &assets, per_hop);
    let amount = per_hop as i128 * ORDER_AMOUNT;
    StellarAssetClient::new(&e, &usd).mint(&trader, &amount);
    let (sold, bought) = client.swap(&direction, &trader, &usd, &amount, &amount, &path);
    assert_eq!(sold, amount);
    assert_eq!(bought, amount);
    let u = measure(
        &e,
        &std::format!("swap {:?} {} hops x {} orders", direction, hops, per_hop),
    );
    assert!(u.within_tx_limits(), "{:?}", u);
}

// ---- fill_order (crank) -------------------------------------------------------------------

/// Taker order sells `n * ORDER_AMOUNT` USD for EUR at 1; `n` makers sell EUR for USD at 1.
fn seed_cross<'a>(
    e: &'a Env,
    usd: &Address,
    eur: &Address,
    n: u32,
) -> (AxisClient<'a>, u64, Vec<u64>) {
    let (client, _, _, maker_ids) = seed_book(e, eur, usd, n, true);
    let owner = Address::generate(e);
    let amount = n as i128 * ORDER_AMOUNT;
    StellarAssetClient::new(e, usd).mint(&owner, &amount);
    let (_, _, taker_id) = client.trade(
        &TradeDirection::Sell,
        &OrderKind::Limit,
        &owner,
        &amount,
        usd,
        eur,
        &PRECISION,
        &Vec::new(e),
    );
    (client, taker_id, maker_ids)
}

#[test]
fn gate_fill_order_cross_8() {
    let (e, _, _, usd, eur) = gate_env();
    let (client, taker_id, maker_ids) = seed_cross(&e, &usd, &eur, 8);
    // The cranker is generated AFTER the contract, so it sorts after the contract address and
    // the contract->cranker leg settles before the cranker->maker leg (see the test below).
    let cranker = Address::generate(&e);
    let (sold, bought) = client.fill_order(&cranker, &taker_id, &maker_ids);
    assert_eq!(sold, 8 * ORDER_AMOUNT);
    assert_eq!(bought, sold);
    let u = measure(
        &e,
        "fill_order crossing 8 maker orders (cranker holds nothing)",
    );
    assert!(u.within_tx_limits(), "{:?}", u);
}

/// Settlement order depends on address ordering (docs/02-design.md, failure modes). The
/// dispatcher executes transfers in `(from, to)` key order; when the cranker's address sorts
/// BEFORE the contract's, the cranker->maker leg runs first and the cranker must front the whole
/// amount. Real G-account crankers always sort before contract addresses (`ScAddress::Account`
/// orders before `ScAddress::Contract`), so this is the production case. Flip this test to a
/// passing one when the flush order is made inflow-first.
#[test]
#[should_panic(expected = "#10")]
fn fill_order_cranker_sorting_before_contract_needs_float() {
    let (e, _, _, usd, eur) = probe_env();
    let cranker = Address::generate(&e); // generated BEFORE the contract is registered
    let (client, taker_id, maker_ids) = seed_cross(&e, &usd, &eur, 1);
    client.fill_order(&cranker, &taker_id, &maker_ids);
}

// ---- views --------------------------------------------------------------------------------

#[test]
fn measure_views() {
    let (e, _, _, usd, eur) = gate_env();
    let (client, _, _, ids) = seed_book(&e, &eur, &usd, 1, true);
    client.order(&ids.get(0).unwrap());
    let u = measure(&e, "order (view)");
    assert_eq!(u.writes, 0);
    client.last();
    let u = measure(&e, "last (view)");
    assert_eq!(u.writes, 0);
}
