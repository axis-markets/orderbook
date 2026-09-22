//! Client interface for the Reflector Beam price oracle.
//! Trimmed to the methods Axis invokes (see `reflector-contract/README.md`, "Beam contract interface").
use soroban_sdk::{contracttype, Address, Symbol, Vec};

/// Oracle contract interface exported as `ReflectorBeamClient`
#[soroban_sdk::contractclient(name = "ReflectorBeamClient")]
#[allow(dead_code)]
pub trait ReflectorBeam {
    /// Number of decimal places used to represent price for all assets quoted by the oracle
    fn decimals() -> u32;
    /// Asset price feed expiration timestamp (panics with `AssetMissing` if the asset is not listed)
    fn expires(asset: Asset) -> Option<u64>;
    /// Most recent price for an asset (requires active tracked access for `caller`)
    fn lastprice(caller: Address, asset: Asset) -> Option<PriceData>;
    /// Purchase access to asset price feeds, the fee token amount is burned from `sponsor`
    /// and split evenly between all `assets`. Returns new access expiration timestamps (in seconds)
    fn track(sponsor: Address, consumer: Address, assets: Vec<Asset>, amount: i128) -> Vec<u64>;
    /// Access expiration UNIX timestamps (in seconds) for the given consumer and assets
    fn tracked_until(consumer: Address, assets: Vec<Asset>) -> Vec<u64>;
    /// Fee token address and daily per-asset price feed fee, `None` when fees are not configured
    fn fee_config() -> FeeConfig;
}

/// Quoted asset definition
#[contracttype]
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum Asset {
    /// Stellar Classic and Soroban assets (token contract address)
    Stellar(Address),
    /// External currencies/tokens/assets/symbols
    Other(Symbol),
}

/// Oracle fee settings
#[contracttype]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FeeConfig {
    /// Fee token contract address and daily per-asset fee amount
    Some((Address, i128)),
    /// Fee settings are not configured
    None,
}

/// Price record definition
#[contracttype]
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct PriceData {
    /// Asset price at given point in time (in oracle decimals)
    pub price: i128,
    /// Record timestamp (in seconds)
    pub timestamp: u64,
}
