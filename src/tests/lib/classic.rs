//! Classic accounts (G addresses) as makers and takers. Every other test module trades between
//! contract addresses, whose token balances know no trustline, reserve or liabilities. Here the
//! ledger holds real account and trustline entries, so the Stellar Asset Contract applies the
//! classic rules: a missing or unauthorized trustline, a trustline limit, buying and selling
//! liabilities, the XLM reserve, an issuer trading its own asset, and an asset whose issuer
//! account is gone. `balance` reports the whole balance, the reserve and the selling liabilities
//! included, so the backing the contract reads can exceed what a transfer can move: such a
//! maker is skipped by `trade` and fails a `swap` or a `crossfill` that pays out of it.
extern crate std;

use super::setup::{
    actor, approve, assert_no_custody, balance, code, list_asset, nonce, register_axis,
    setup_oracle, store_order, trade, try_trade, START_TIMESTAMP, UNIT_PRICE,
};
use crate::events::OrderSkippedEvent;
use crate::order::{order_id, Order, OrderKind, TradeDirection};
use crate::trade::TradeStep;
use crate::{orderbook::PRECISION, AxisClient};
use core::sync::atomic::{AtomicU64, Ordering};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{token, xdr, Address, Env, Event, TryFromVal, Vec};
use std::rc::Rc;

/// One XLM in stroops
const XLM: i64 = 10_000_000;
/// Base reserve of the test ledger: 0.5 XLM, as on mainnet
const BASE_RESERVE: u32 = 5_000_000;
/// Trustline flag of a holder the issuer authorized
const AUTHORIZED: u32 = xdr::TrustLineFlags::AuthorizedFlag as u32;
/// Trustline flag of a holder who may only keep the offers already placed
const MAINTAIN_LIABILITIES: u32 = xdr::TrustLineFlags::AuthorizedToMaintainLiabilitiesFlag as u32;
/// Token error for a balance a transfer cannot take or exceed
const BALANCE_ERROR: u32 = 10;

static SEED: AtomicU64 = AtomicU64::new(1);

/// A Stellar asset: its token contract and the classic asset the trustlines refer to
struct Token {
    address: Address,
    asset: xdr::Asset,
}

/// Trustline fields a test sets
#[derive(Clone, Copy)]
struct Line {
    balance: i64,
    limit: i64,
    flags: u32,
    buying: i64,
    selling: i64,
}

/// An authorized trustline holding `balance`, without a limit or liabilities
fn line(balance: i64) -> Line {
    Line {
        balance,
        limit: i64::MAX,
        flags: AUTHORIZED,
        buying: 0,
        selling: 0,
    }
}

/// Test ledger with a base reserve, the oracle and the AXIS contract. Returns the contract and
/// the address that administers the test assets
fn setup() -> (Env, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    e.ledger().set_timestamp(START_TIMESTAMP);
    e.ledger().set_base_reserve(BASE_RESERVE);
    let admin = Address::generate(&e);
    setup_oracle(&e, &admin);
    let axis = register_axis(&e);
    (e, axis, admin)
}

/// Write a ledger entry, replacing the one stored under the key
fn put_entry(e: &Env, key: xdr::LedgerKey, data: xdr::LedgerEntryData) {
    let entry = xdr::LedgerEntry {
        last_modified_ledger_seq: 0,
        data,
        ext: xdr::LedgerEntryExt::V0,
    };
    e.host()
        .add_ledger_entry(&Rc::new(key), &Rc::new(entry), None)
        .unwrap();
}

/// Deploy the token contract of a classic asset and list it on the oracle at unit price
fn deploy(e: &Env, asset: xdr::Asset) -> Token {
    let create = xdr::HostFunction::CreateContract(xdr::CreateContractArgs {
        contract_id_preimage: xdr::ContractIdPreimage::Asset(asset.clone()),
        executable: xdr::ContractExecutable::StellarAsset,
    });
    let address = Address::try_from_val(e, &e.host().invoke_function(create).unwrap()).unwrap();
    list_asset(e, &address, UNIT_PRICE);
    Token { address, asset }
}

/// An asset issued by a fresh issuer account, listed on the oracle at unit price
fn credit(e: &Env, admin: &Address) -> Token {
    let sac = e.register_stellar_asset_contract_v2(admin.clone());
    list_asset(e, &sac.address(), UNIT_PRICE);
    Token {
        address: sac.address(),
        asset: sac.asset(),
    }
}

/// The native asset
fn native(e: &Env) -> Token {
    deploy(e, xdr::Asset::Native)
}

/// An asset whose issuer account does not exist, as after the issuer merged the account
fn orphan(e: &Env) -> Token {
    deploy(
        e,
        xdr::Asset::CreditAlphanum4(xdr::AlphaNum4 {
            asset_code: xdr::AssetCode4(*b"ORPH"),
            issuer: new_account_id(),
        }),
    )
}

/// A key no account of the test ledger uses yet
fn new_account_id() -> xdr::AccountId {
    let mut key = [0u8; 32];
    key[..8].copy_from_slice(&SEED.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    xdr::AccountId(xdr::PublicKey::PublicKeyTypeEd25519(xdr::Uint256(key)))
}

fn account_id(account: &Address) -> xdr::AccountId {
    match xdr::ScAddress::from(account) {
        xdr::ScAddress::Account(id) => id,
        other => panic!("not a classic account: {:?}", other),
    }
}

fn address_of(e: &Env, id: &xdr::AccountId) -> Address {
    Address::try_from_val(e, &xdr::ScAddress::Account(id.clone())).unwrap()
}

/// Account of the asset's issuer
fn issuer_of(e: &Env, token: &Token) -> Address {
    match &token.asset {
        xdr::Asset::CreditAlphanum4(asset) => address_of(e, &asset.issuer),
        other => panic!("no issuer: {:?}", other),
    }
}

/// Write the account entry: `xlm` stroops, `sub_entries` entries counted towards the reserve
/// and `selling` stroops of XLM locked in classic offers
fn set_account(e: &Env, account: &Address, xlm: i64, sub_entries: u32, selling: i64) {
    let id = account_id(account);
    let ext = if selling == 0 {
        xdr::AccountEntryExt::V0
    } else {
        xdr::AccountEntryExt::V1(xdr::AccountEntryExtensionV1 {
            liabilities: xdr::Liabilities { buying: 0, selling },
            ext: xdr::AccountEntryExtensionV1Ext::V0,
        })
    };
    put_entry(
        e,
        xdr::LedgerKey::Account(xdr::LedgerKeyAccount {
            account_id: id.clone(),
        }),
        xdr::LedgerEntryData::Account(xdr::AccountEntry {
            account_id: id,
            balance: xlm,
            seq_num: xdr::SequenceNumber(0),
            num_sub_entries: sub_entries,
            inflation_dest: None,
            flags: 0,
            home_domain: Default::default(),
            thresholds: xdr::Thresholds([1; 4]),
            signers: xdr::VecM::default(),
            ext,
        }),
    );
}

/// A new classic account holding `xlm` stroops, with `sub_entries` entries counted towards the
/// reserve
fn classic_account(e: &Env, xlm: i64, sub_entries: u32) -> Address {
    let account = address_of(e, &new_account_id());
    set_account(e, &account, xlm, sub_entries, 0);
    account
}

/// Write the account's trustline for the asset
fn set_line(e: &Env, account: &Address, token: &Token, line: Line) {
    let id = account_id(account);
    let asset = match token.asset.clone() {
        xdr::Asset::CreditAlphanum4(asset) => xdr::TrustLineAsset::CreditAlphanum4(asset),
        xdr::Asset::CreditAlphanum12(asset) => xdr::TrustLineAsset::CreditAlphanum12(asset),
        xdr::Asset::Native => panic!("XLM is held by the account itself"),
    };
    let ext = if line.buying == 0 && line.selling == 0 {
        xdr::TrustLineEntryExt::V0
    } else {
        xdr::TrustLineEntryExt::V1(xdr::TrustLineEntryV1 {
            liabilities: xdr::Liabilities {
                buying: line.buying,
                selling: line.selling,
            },
            ext: xdr::TrustLineEntryV1Ext::V0,
        })
    };
    put_entry(
        e,
        xdr::LedgerKey::Trustline(xdr::LedgerKeyTrustLine {
            account_id: id.clone(),
            asset: asset.clone(),
        }),
        xdr::LedgerEntryData::Trustline(xdr::TrustLineEntry {
            account_id: id,
            asset,
            balance: line.balance,
            limit: line.limit,
            flags: line.flags,
            ext,
        }),
    );
}

/// A classic account with 100 XLM holding `amount` of `holds` and an empty trustline for
/// `receives`, both spendable by the contract
fn classic_trader(
    e: &Env,
    axis: &Address,
    holds: &Token,
    amount: i64,
    receives: &Token,
) -> Address {
    let account = classic_account(e, 100 * XLM, 2);
    set_line(e, &account, holds, line(amount));
    set_line(e, &account, receives, line(0));
    approve(e, &holds.address, axis, &account, i128::MAX);
    approve(e, &receives.address, axis, &account, i128::MAX);
    account
}

/// Write a maker order straight into the contract storage. The test ledger cannot delete an
/// entry, so an order whose owner closed a trustline after creating it is written the way it
/// would be left on the book
fn book_order(
    e: &Env,
    axis: &Address,
    owner: &Address,
    amount: i128,
    selling: &Token,
    buying: &Token,
) -> u128 {
    e.as_contract(axis, || {
        let id = order_id(e, owner, nonce());
        let order = Order {
            id,
            selling: selling.address.clone(),
            buying: buying.address.clone(),
            amount,
            owner: owner.clone(),
            price: PRECISION,
            expires: 0,
        };
        crate::order::store_order(e, &order, false);
        id
    })
}

/// The taker sells `amount` of `selling` to the listed order at the unit price, taking what
/// fills
fn fill(
    client: &AxisClient,
    taker: &Address,
    amount: i128,
    selling: &Token,
    buying: &Token,
    id: u128,
) -> (i128, i128, Option<u128>) {
    trade(
        client,
        TradeDirection::Sell,
        OrderKind::Fill,
        taker,
        amount,
        &selling.address,
        &buying.address,
        PRECISION,
        &Vec::from_array(&client.env, [id]),
    )
}

/// `fill` returning the error code of a failed call
fn try_fill(
    client: &AxisClient,
    taker: &Address,
    amount: i128,
    selling: &Token,
    buying: &Token,
    id: u128,
) -> Option<u32> {
    try_trade(
        client,
        TradeDirection::Sell,
        OrderKind::Fill,
        taker,
        amount,
        &selling.address,
        &buying.address,
        PRECISION,
        &Vec::from_array(&client.env, [id]),
    )
}

/// The call emitted a single `skip` event for the order
fn assert_skipped(e: &Env, axis: &Address, id: u128) {
    assert_eq!(
        e.events().all().filter_by_contract(axis),
        [OrderSkippedEvent { order: id }.to_xdr(e, axis)]
    );
}

#[test]
fn test_classic_accounts_trade_through_trustlines() {
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_trader(&e, &axis, &eur, 10000, &usd);
    let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
    let id = store_order(&client, &maker, 1000, &eur.address, &usd.address, PRECISION);

    assert_eq!(fill(&client, &taker, 400, &usd, &eur, id), (400, 400, None));
    assert_eq!(client.order(&id).unwrap().amount, 600);
    assert_eq!(balance(&e, &eur.address, &maker), 9600);
    assert_eq!(balance(&e, &usd.address, &maker), 400);
    assert_eq!(balance(&e, &usd.address, &taker), 9600);
    assert_eq!(balance(&e, &eur.address, &taker), 400);
    assert_no_custody(&e, &axis, &[&usd.address, &eur.address]);
}

#[test]
fn test_taker_without_a_trustline_for_the_bought_asset_is_rejected() {
    // the token answers `authorized` with its own error for a missing trustline, which reads as
    // "cannot receive": the trade fails before anything moves and no maker is flagged
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_trader(&e, &axis, &eur, 10000, &usd);
    let taker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &taker, &usd, line(10000));
    approve(&e, &usd.address, &axis, &taker, i128::MAX);
    let id = store_order(&client, &maker, 1000, &eur.address, &usd.address, PRECISION);

    assert_eq!(try_fill(&client, &taker, 400, &usd, &eur, id), Some(708));
    // an order buying the asset cannot be created either
    assert_eq!(
        try_trade(
            &client,
            TradeDirection::Sell,
            OrderKind::Limit,
            &taker,
            400,
            &usd.address,
            &eur.address,
            PRECISION,
            &Vec::new(&e),
        ),
        Some(708)
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
    assert_eq!(balance(&e, &eur.address, &maker), 10000);
}

#[test]
fn test_maker_without_a_trustline_for_the_payment_is_skipped() {
    // the maker closed the trustline for the asset the order buys
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &maker, &eur, line(10000));
    approve(&e, &eur.address, &axis, &maker, i128::MAX);
    let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
    let id = book_order(&e, &axis, &maker, 1000, &eur, &usd);

    assert_eq!(fill(&client, &taker, 400, &usd, &eur, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &eur.address, &maker), 10000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
}

#[test]
fn test_maker_without_a_trustline_for_the_sold_asset_is_skipped() {
    // the maker closed the trustline for the asset the order sells: the balance cannot be read
    // and counts as zero, while the allowance granted earlier still reads in full
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &maker, &usd, line(0));
    approve(&e, &eur.address, &axis, &maker, 5000);
    let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
    let id = book_order(&e, &axis, &maker, 1000, &eur, &usd);
    assert_eq!(
        token::Client::new(&e, &eur.address).allowance(&maker, &axis),
        5000
    );

    assert_eq!(fill(&client, &taker, 400, &usd, &eur, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
}

#[test]
fn test_maker_with_an_unauthorized_trustline_for_the_payment_is_skipped() {
    // the issuer revoked the maker's trustline for the asset the order buys, fully or down to
    // maintaining the offers already placed: either way the maker cannot receive
    for flags in [0, MAINTAIN_LIABILITIES] {
        let (e, axis, admin) = setup();
        let client = AxisClient::new(&e, &axis);
        let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
        let maker = classic_trader(&e, &axis, &eur, 10000, &usd);
        let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
        let id = store_order(&client, &maker, 1000, &eur.address, &usd.address, PRECISION);
        set_line(&e, &maker, &usd, Line { flags, ..line(0) });

        assert_eq!(fill(&client, &taker, 400, &usd, &eur, id), (0, 0, None));
        assert_skipped(&e, &axis, id);
        assert_eq!(client.order(&id).unwrap().amount, 1000);
        assert_eq!(balance(&e, &usd.address, &taker), 10000);
    }
}

#[test]
fn test_maker_who_may_only_maintain_liabilities_cannot_deliver() {
    // the maker's trustline for the asset the order sells still reads its balance, but the
    // token refuses to move it: the maker is skipped at settlement
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_trader(&e, &axis, &eur, 10000, &usd);
    let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
    let id = store_order(&client, &maker, 1000, &eur.address, &usd.address, PRECISION);
    set_line(
        &e,
        &maker,
        &eur,
        Line {
            flags: MAINTAIN_LIABILITIES,
            ..line(10000)
        },
    );
    assert_eq!(balance(&e, &eur.address, &maker), 10000);

    assert_eq!(fill(&client, &taker, 400, &usd, &eur, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
    assert_no_custody(&e, &axis, &[&usd.address, &eur.address]);
}

#[test]
fn test_maker_trustline_limit_and_buying_liabilities_fail_the_payment() {
    // the maker's trustline for the payment takes 1000 at most and 400 of that is reserved for
    // classic offers, leaving room for 600. `authorized` shows none of it, so the maker is
    // admitted and the payment fails the trade with the token's own error
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_trader(&e, &axis, &eur, 10000, &usd);
    let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
    let id = store_order(&client, &maker, 1000, &eur.address, &usd.address, PRECISION);
    set_line(
        &e,
        &maker,
        &usd,
        Line {
            limit: 1000,
            buying: 400,
            ..line(0)
        },
    );

    assert_eq!(
        try_fill(&client, &taker, 601, &usd, &eur, id),
        Some(BALANCE_ERROR)
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &eur.address, &maker), 10000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
    // a fill that fits the room left settles
    assert_eq!(fill(&client, &taker, 600, &usd, &eur, id), (600, 600, None));
    assert_eq!(balance(&e, &usd.address, &maker), 600);
    assert_no_custody(&e, &axis, &[&usd.address, &eur.address]);
}

#[test]
fn test_maker_selling_liabilities_cannot_be_collected() {
    // 600 of the maker's 1000 EUR are locked in classic offers. `balance` reports all 1000, so
    // the order is backed in full as far as the contract can tell, but the token moves 400 at
    // most: `trade` skips the maker, the strict `swap` fails
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let maker = classic_trader(&e, &axis, &eur, 1000, &usd);
    let taker = classic_trader(&e, &axis, &usd, 10000, &eur);
    set_line(
        &e,
        &maker,
        &eur,
        Line {
            selling: 600,
            ..line(1000)
        },
    );
    assert_eq!(balance(&e, &eur.address, &maker), 1000);
    let id = store_order(&client, &maker, 1000, &eur.address, &usd.address, PRECISION);

    assert_eq!(fill(&client, &taker, 401, &usd, &eur, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: eur.address.clone(),
            orders: Vec::from_array(&e, [id]),
        }],
    );
    assert_eq!(
        code(client.try_swap(
            &TradeDirection::Sell,
            &taker,
            &usd.address,
            &401,
            &401,
            &path,
            &None,
        )),
        Some(BALANCE_ERROR)
    );
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
    // what the offers leave free trades
    assert_eq!(fill(&client, &taker, 400, &usd, &eur, id), (400, 400, None));
    assert_eq!(balance(&e, &eur.address, &maker), 600);
    assert_eq!(client.order(&id).unwrap().amount, 600);
    assert_no_custody(&e, &axis, &[&usd.address, &eur.address]);
}

#[test]
fn test_xlm_reserve_of_a_maker_cannot_be_collected() {
    // the maker's account holds 100 XLM, 1.5 of which are the reserve for the account and its
    // trustline. `balance` reports all 100, so an order selling them all is accepted, but the
    // reserve never moves
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (xlm, usd) = (native(&e), credit(&e, &admin));
    let reserve = 3 * BASE_RESERVE as i128;
    let all = 100 * XLM as i128;
    let maker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &maker, &usd, line(0));
    approve(&e, &xlm.address, &axis, &maker, i128::MAX);
    let taker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &taker, &usd, line(200 * XLM));
    approve(&e, &usd.address, &axis, &taker, i128::MAX);
    assert_eq!(balance(&e, &xlm.address, &maker), all);
    let id = store_order(&client, &maker, all, &xlm.address, &usd.address, PRECISION);

    // taking the whole order needs the reserve
    assert_eq!(fill(&client, &taker, all, &usd, &xlm, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_eq!(client.order(&id).unwrap().amount, all);
    // everything above the reserve trades
    let free = all - reserve;
    assert_eq!(
        fill(&client, &taker, free, &usd, &xlm, id),
        (free, free, None)
    );
    assert_eq!(balance(&e, &xlm.address, &maker), reserve);
    assert_eq!(balance(&e, &xlm.address, &taker), all + free);
    assert_eq!(client.order(&id).unwrap().amount, reserve);
    // the remainder is the reserve itself
    assert_eq!(fill(&client, &taker, reserve, &usd, &xlm, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_no_custody(&e, &axis, &[&usd.address, &xlm.address]);
}

#[test]
fn test_xlm_reserve_and_selling_liabilities_of_a_taker_fail_the_payment() {
    // the taker holds 100 XLM: 1.5 are the reserve and 50 are locked in classic offers, so
    // 48.5 can be paid. A payment above that fails the trade with the token's own error and
    // does not flag the maker
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (xlm, usd) = (native(&e), credit(&e, &admin));
    let free = (100 * XLM - 3 * BASE_RESERVE as i64 - 50 * XLM) as i128;
    let maker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &maker, &usd, line(200 * XLM));
    approve(&e, &usd.address, &axis, &maker, i128::MAX);
    let taker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &taker, &usd, line(0));
    set_account(&e, &taker, 100 * XLM, 1, 50 * XLM);
    approve(&e, &xlm.address, &axis, &taker, i128::MAX);
    let id = store_order(
        &client,
        &maker,
        100 * XLM as i128,
        &usd.address,
        &xlm.address,
        PRECISION,
    );

    assert_eq!(
        try_fill(&client, &taker, free + 1, &xlm, &usd, id),
        Some(BALANCE_ERROR)
    );
    assert_eq!(client.order(&id).unwrap().amount, 100 * XLM as i128);
    assert_eq!(balance(&e, &usd.address, &maker), 200 * XLM as i128);
    assert_eq!(
        fill(&client, &taker, free, &xlm, &usd, id),
        (free, free, None)
    );
    assert_eq!(balance(&e, &usd.address, &taker), free);
    assert_eq!(balance(&e, &xlm.address, &taker), 100 * XLM as i128 - free);
    assert_no_custody(&e, &axis, &[&usd.address, &xlm.address]);
}

#[test]
fn test_crossfill_paying_out_of_the_xlm_reserve_fails() {
    // the taker order sells the owner's whole XLM balance. The owner pays the makers with a
    // plain transfer, so a crossfill that reaches into the reserve fails instead of skipping,
    // and one that stays above it settles
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (xlm, usd) = (native(&e), credit(&e, &admin));
    let reserve = 3 * BASE_RESERVE as i128;
    let all = 100 * XLM as i128;
    let owner = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &owner, &usd, line(0));
    approve(&e, &xlm.address, &axis, &owner, i128::MAX);
    let maker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &maker, &usd, line(200 * XLM));
    approve(&e, &usd.address, &axis, &maker, i128::MAX);
    let cranker = actor(&e);
    let taker_order = store_order(&client, &owner, all, &xlm.address, &usd.address, PRECISION);
    let whole = store_order(&client, &maker, all, &usd.address, &xlm.address, PRECISION);
    let partial = store_order(
        &client,
        &maker,
        all - reserve,
        &usd.address,
        &xlm.address,
        PRECISION,
    );

    assert_eq!(
        code(client.try_crossfill(&cranker, &taker_order, &Vec::from_array(&e, [whole]))),
        Some(BALANCE_ERROR)
    );
    assert_eq!(client.order(&taker_order).unwrap().amount, all);
    assert_eq!(
        client.crossfill(&cranker, &taker_order, &Vec::from_array(&e, [partial])),
        (all - reserve, all - reserve, 0)
    );
    assert_eq!(balance(&e, &xlm.address, &owner), reserve);
    assert_eq!(balance(&e, &usd.address, &owner), all - reserve);
    assert_eq!(client.order(&taker_order).unwrap().amount, reserve);
    assert_no_custody(&e, &axis, &[&usd.address, &xlm.address]);
}

#[test]
fn test_swap_between_classic_accounts_needs_no_trustline_for_the_hop_asset() {
    // USD -> EUR -> XLM: the EUR passes through the contract only, so the trader holds no EUR
    // trustline
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (xlm, usd, eur) = (native(&e), credit(&e, &admin), credit(&e, &admin));
    let eur_maker = classic_trader(&e, &axis, &eur, 10000, &usd);
    let xlm_maker = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &xlm_maker, &eur, line(0));
    approve(&e, &xlm.address, &axis, &xlm_maker, i128::MAX);
    let trader = classic_account(&e, 100 * XLM, 1);
    set_line(&e, &trader, &usd, line(10000));
    approve(&e, &usd.address, &axis, &trader, i128::MAX);
    let eur_order = store_order(
        &client,
        &eur_maker,
        1000,
        &eur.address,
        &usd.address,
        PRECISION,
    );
    let xlm_order = store_order(
        &client,
        &xlm_maker,
        1000,
        &xlm.address,
        &eur.address,
        PRECISION,
    );
    let path = Vec::from_array(
        &e,
        [
            TradeStep {
                asset: eur.address.clone(),
                orders: Vec::from_array(&e, [eur_order]),
            },
            TradeStep {
                asset: xlm.address.clone(),
                orders: Vec::from_array(&e, [xlm_order]),
            },
        ],
    );

    assert_eq!(
        client.swap(
            &TradeDirection::Sell,
            &trader,
            &usd.address,
            &1000,
            &1000,
            &path,
            &None,
        ),
        (1000, 1000)
    );
    assert_eq!(balance(&e, &usd.address, &trader), 9000);
    assert_eq!(balance(&e, &xlm.address, &trader), 100 * XLM as i128 + 1000);
    assert_eq!(balance(&e, &usd.address, &eur_maker), 1000);
    assert_eq!(balance(&e, &eur.address, &eur_maker), 9000);
    assert_eq!(balance(&e, &eur.address, &xlm_maker), 1000);
    assert_eq!(
        balance(&e, &xlm.address, &xlm_maker),
        100 * XLM as i128 - 1000
    );
    assert_no_custody(&e, &axis, &[&usd.address, &eur.address, &xlm.address]);
}

#[test]
fn test_issuer_account_issues_and_redeems_through_fills() {
    // the issuer holds no balance of its own asset: an order selling it is always backed, a
    // fill issues new units and a fill the issuer buys redeems them
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, eur) = (credit(&e, &admin), credit(&e, &admin));
    let issuer = issuer_of(&e, &eur);
    set_account(&e, &issuer, 100 * XLM, 1, 0);
    set_line(&e, &issuer, &usd, line(0));
    approve(&e, &eur.address, &axis, &issuer, i128::MAX);
    approve(&e, &usd.address, &axis, &issuer, i128::MAX);
    let holder = classic_trader(&e, &axis, &usd, 10000, &eur);
    let unlimited = i64::MAX as i128;
    assert_eq!(balance(&e, &eur.address, &issuer), unlimited);

    // the holder buys 1000 EUR from the issuer
    let issue = store_order(
        &client,
        &issuer,
        1000,
        &eur.address,
        &usd.address,
        PRECISION,
    );
    assert_eq!(
        fill(&client, &holder, 1000, &usd, &eur, issue),
        (1000, 1000, None)
    );
    assert_eq!(balance(&e, &eur.address, &holder), 1000);
    assert_eq!(balance(&e, &usd.address, &issuer), 1000);
    assert_eq!(balance(&e, &eur.address, &issuer), unlimited);
    // the issuer buys 400 EUR back from the holder
    let redeem = store_order(
        &client,
        &holder,
        1000,
        &eur.address,
        &usd.address,
        PRECISION,
    );
    assert_eq!(
        fill(&client, &issuer, 400, &usd, &eur, redeem),
        (400, 400, None)
    );
    assert_eq!(balance(&e, &eur.address, &holder), 600);
    assert_eq!(balance(&e, &usd.address, &issuer), 600);
    assert_eq!(balance(&e, &eur.address, &issuer), unlimited);
    assert_no_custody(&e, &axis, &[&usd.address, &eur.address]);
}

/// A maker selling 1000 units of an asset whose issuer account is gone, a taker able to hold
/// it, and the maker's order
fn merged_issuer_book(
    e: &Env,
    axis: &Address,
    client: &AxisClient,
    admin: &Address,
) -> (Token, Token, Address, Address, u128) {
    let (usd, orph) = (credit(e, admin), orphan(e));
    let maker = classic_trader(e, axis, &orph, 1000, &usd);
    let taker = classic_trader(e, axis, &usd, 10000, &orph);
    let id = store_order(client, &maker, 1000, &orph.address, &usd.address, PRECISION);
    (usd, orph, maker, taker, id)
}

#[test]
fn test_trade_into_an_asset_of_a_merged_issuer_skips_the_maker() {
    // The contract has never held the asset, so the token has to read the issuer's flags to
    // tell whether the contract may hold it, and the issuer account is gone. `authorized` fails
    // without an answer from the token, which `can_receive` reads as "can receive": the trade
    // passes the up-front check, then the maker's transfer into the contract fails for the same
    // reason. Nothing moves, but the maker is reported with a `skip` event for a fault that is
    // not theirs
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, orph, maker, taker, id) = merged_issuer_book(&e, &axis, &client, &admin);
    let token = token::StellarAssetClient::new(&e, &orph.address);
    // the token cannot say whether the contract may hold the asset, holders with a trustline
    // read fine
    let unanswered = token.try_authorized(&axis);
    assert!(
        matches!(unanswered, Err(Ok(err)) if !err.is_type(ScErrorType::Contract)),
        "{:?}",
        unanswered
    );
    assert_eq!(token.try_authorized(&maker), Ok(Ok(true)));
    assert_eq!(token.try_authorized(&taker), Ok(Ok(true)));

    assert_eq!(fill(&client, &taker, 400, &usd, &orph, id), (0, 0, None));
    assert_skipped(&e, &axis, id);
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &orph.address, &maker), 1000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
    assert_no_custody(&e, &axis, &[&usd.address, &orph.address]);
}

#[test]
fn test_swap_into_an_asset_of_a_merged_issuer_fails() {
    // the strict settlement of `swap` fails on the maker's transfer into the contract instead
    // of skipping the maker
    let (e, axis, admin) = setup();
    let client = AxisClient::new(&e, &axis);
    let (usd, orph, maker, taker, id) = merged_issuer_book(&e, &axis, &client, &admin);
    let path = Vec::from_array(
        &e,
        [TradeStep {
            asset: orph.address.clone(),
            orders: Vec::from_array(&e, [id]),
        }],
    );

    let res = client.try_swap(
        &TradeDirection::Sell,
        &taker,
        &usd.address,
        &400,
        &400,
        &path,
        &None,
    );
    // neither a contract error of the token nor one of AXIS (712 would name the contract)
    assert!(
        matches!(res, Err(Ok(err)) if !err.is_type(ScErrorType::Contract)),
        "{:?}",
        res
    );
    assert_eq!(client.order(&id).unwrap().amount, 1000);
    assert_eq!(balance(&e, &orph.address, &maker), 1000);
    assert_eq!(balance(&e, &usd.address, &taker), 10000);
}
