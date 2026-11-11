//! Calls whose outcome depends on where they are made from.
//!
//! `requote` checks no signature, so another contract can call it. A contract account runs its
//! `__check_auth` while the contract asking for the authorization is still on the call stack, so
//! an account asked to authorize an oracle call can requote a market with the oracle already on
//! the stack. The host then refuses the nested oracle calls with a recoverable error, which
//! `resolve_side` must not read as "not quoted": the requote fails instead of delisting the market.
extern crate std;

use super::setup::{
    actor, code, fund, oracle_address, oracle_client, order_update, register_axis, setup_test,
    store_order, trade, try_update_one,
};
use crate::math::MAX_PRICE;
use crate::order::{OrderKind, TradeDirection};
use crate::orderbook::PRECISION;
use crate::reflector_beam::Asset;
use crate::AxisClient;
use soroban_sdk::auth::{Context, CustomAccountInterface};
use soroban_sdk::crypto::Hash;
use soroban_sdk::xdr::{
    InvokeContractArgs, ScAddress, ScSymbol, ScVal, SorobanAddressCredentials,
    SorobanAuthorizationEntry, SorobanAuthorizedFunction, SorobanAuthorizedInvocation,
    SorobanCredentials, VecM,
};
use soroban_sdk::{
    contract, contracterror, contractimpl, symbol_short, Address, Env, IntoVal, Val, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HostileError {
    Denied = 1,
}

/// Account contract that requotes a market while it is asked to authorize a call, and approves
/// the call whatever the requote did, so the transaction around it succeeds. Its "signature" is
/// `[axis, base, quote]`
#[contract]
pub struct HostileAccount;

#[contractimpl]
impl HostileAccount {
    /// Error code of the last nested requote, 0 when it succeeded
    pub fn requote_code(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("code"))
            .unwrap()
    }
}

#[contractimpl]
impl CustomAccountInterface for HostileAccount {
    type Signature = Vec<Address>;
    type Error = HostileError;

    #[allow(non_snake_case)]
    fn __check_auth(
        env: Env,
        _signature_payload: Hash<32>,
        signature: Vec<Address>,
        _auth_contexts: Vec<Context>,
    ) -> Result<(), HostileError> {
        let axis = signature.get(0).ok_or(HostileError::Denied)?;
        let base = signature.get(1).ok_or(HostileError::Denied)?;
        let quote = signature.get(2).ok_or(HostileError::Denied)?;
        let code = match AxisClient::new(&env, &axis).try_requote(&base, &quote) {
            Ok(_) => 0,
            Err(Ok(err)) => err.get_code(),
            Err(Err(_)) => u32::MAX,
        };
        env.storage().instance().set(&symbol_short!("code"), &code);
        Ok(())
    }
}

/// Control vehicle: any contract that asks the account to authorize, without the oracle on the
/// call stack
#[contract]
pub struct Vehicle;

#[contractimpl]
impl Vehicle {
    pub fn poke(_e: Env, account: Address) {
        account.require_auth();
    }
}

/// Whether the host recorded its re-entry refusal during the latest top-level call
fn reentry_refused(e: &Env) -> bool {
    let events = e.host().get_diagnostic_events().unwrap();
    std::format!("{:?}", events).contains("Contract re-entry is not allowed")
}

/// Authorization entry of `account` for `contract.function(args)`, carrying the market to
/// requote as its signature
fn auth_entry(
    e: &Env,
    account: &Address,
    contract: &Address,
    function: &str,
    args: Vec<Val>,
    signature: Vec<Address>,
) -> SorobanAuthorizationEntry {
    let ScVal::Vec(Some(args)) = ScVal::from(args) else {
        panic!("arguments must convert to a vector")
    };
    SorobanAuthorizationEntry {
        credentials: SorobanCredentials::Address(SorobanAddressCredentials {
            address: ScAddress::from(account),
            nonce: 1,
            signature_expiration_ledger: e.ledger().sequence() + 100,
            signature: ScVal::from(signature),
        }),
        root_invocation: SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: ScAddress::from(contract),
                function_name: ScSymbol(function.try_into().unwrap()),
                args: args.0,
            }),
            sub_invocations: VecM::default(),
        },
    }
}

#[test]
fn test_requote_nested_in_an_oracle_call_cannot_delist_the_market() {
    let (e, trader, _issuer, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10_000);
    fund(&e, &eur, &axis, &trader, 10_000);
    let order = store_order(&client, &maker, 1000, &usd, &eur, PRECISION);
    let listed = client.market(&usd, &eur).unwrap();
    assert!(listed.base.listed && listed.quote.listed);

    //the attacker: an account contract with read access to some oracle feed. On Reflector Beam
    //the base asset is readable by everyone, so the access costs nothing there
    let hostile = e.register(HostileAccount, ());
    let feed = Asset::Stellar(usd.clone());
    oracle_client(&e).set_access(&hostile, &feed, &u64::MAX);

    //`lastprice` asks the account to authorize, the account requotes the market from inside
    let target = Vec::from_array(&e, [axis.clone(), usd.clone(), eur.clone()]);
    let args: Vec<Val> = (hostile.clone(), feed.clone()).into_val(&e);
    let oracle = oracle_address(&e);
    e.set_auths(&[auth_entry(&e, &hostile, &oracle, "lastprice", args, target)]);
    oracle_client(&e).lastprice(&hostile, &feed);
    //the host refused the nested oracle calls: the oracle was already on the call stack
    assert!(reentry_refused(&e));
    //no answer from the oracle fails the requote instead of delisting the assets
    assert_eq!(HostileAccountClient::new(&e, &hostile).requote_code(), 721);
    e.mock_all_auths();

    //the market record is untouched and keeps taking new orders and order changes
    assert_eq!(client.market(&usd, &eur).unwrap(), listed);
    let (_, _, id) = trade(
        &client,
        TradeDirection::Sell,
        OrderKind::Limit,
        &trader,
        1000,
        &eur,
        &usd,
        PRECISION * 2,
        &Vec::new(&e),
    );
    assert!(id.is_some());
    assert_eq!(
        try_update_one(&client, &maker, order_update(order, 900, 2 * PRECISION)),
        None
    );
}

#[test]
fn test_requote_nested_in_another_call_checks_the_market() {
    //control: the same attacker code, run while a contract other than the oracle asks it to
    //authorize. The nested requote reaches the oracle and succeeds, so the failure above comes
    //from the oracle being on the call stack, not from requoting inside `__check_auth`
    let (e, _trader, _issuer, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    fund(&e, &usd, &axis, &maker, 10_000);
    store_order(&client, &maker, 1000, &usd, &eur, PRECISION);

    let hostile = e.register(HostileAccount, ());
    let vehicle = e.register(Vehicle, ());
    let target = Vec::from_array(&e, [axis.clone(), usd.clone(), eur.clone()]);
    let args: Vec<Val> = (hostile.clone(),).into_val(&e);
    e.set_auths(&[auth_entry(&e, &hostile, &vehicle, "poke", args, target)]);
    VehicleClient::new(&e, &vehicle).poke(&hostile);
    assert!(!reentry_refused(&e));
    assert_eq!(HostileAccountClient::new(&e, &hostile).requote_code(), 0);
    e.mock_all_auths();

    let market = client.market(&usd, &eur).unwrap();
    assert!(market.base.listed && market.quote.listed);
}

#[test]
fn test_unfilled_fill_or_kill_reports_overflow_for_a_huge_leftover() {
    //the leftover check converts what is left at the highest order price filled; a result
    //beyond i128 fails with `Overflow` (740) where the trade is simply not filled (709)
    let (e, trader, _issuer, usd, eur) = setup_test();
    let axis = register_axis(&e);
    let client = AxisClient::new(&e, &axis);
    let maker = actor(&e);
    let amount = 10i128.pow(16);
    fund(&e, &usd, &axis, &maker, amount);
    //lowest price: the whole order costs one unit of EUR
    let order = store_order(&client, &maker, amount, &usd, &eur, 100);
    let fill_or_kill = |selling: i128| {
        code(client.try_trade(
            &TradeDirection::Sell,
            &OrderKind::FillOrKill,
            &trader,
            &selling,
            &eur,
            &usd,
            &MAX_PRICE,
            &Vec::from_array(&e, [order]),
            &1,
            &0,
            &None,
        ))
    };
    assert_eq!(fill_or_kill(10i128.pow(20)), Some(709));
    assert_eq!(fill_or_kill(10i128.pow(24)), Some(740));
}
