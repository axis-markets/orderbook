use soroban_sdk::contracterror;

/// Standard contract errors
#[contracterror]
#[repr(i16)]
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderbookError {
    /// Only order owner can modify it
    NotAuthorized = 701,
    /// Insufficient balance on the trader account
    InsufficientBalance = 702,
    /// The allowance granted to the contract does not cover the order
    InsufficientAllowance = 703,
    /// Invalid match parameters (assets, path, etc.)
    InvalidMatch = 704,
    /// Price is out of range
    InvalidPrice = 705,
    /// Trade amount or a configuration value is out of range
    InvalidAmount = 706,
    /// The order expiration timestamp is not in the future
    InvalidExpiration = 707,
    /// The receiving account cannot accept the asset (missing or deauthorized trustline)
    CannotReceive = 708,
    /// The trade cannot be executed in full (`FillOrKill` trades and `swap` bounds)
    NotFilled = 709,
    /// Order with a given ID was not found
    OrderNotFound = 710,
    /// A live order with the same id already exists
    OrderExists = 711,
    /// The contract cannot hold an asset it passes on (in `trade`, `crossfill` or `swap` hops)
    IntermediaryCannotReceive = 712,
    /// Order value is below the minimum order size
    OrderSizeTooSmall = 720,
    /// The market does not exist, or none of its assets is listed on the price oracle
    AssetsNotVerifiedByOracle = 721,
    /// No listed market asset has a usable cached price (none was cached within the last 72 hours)
    AssetPriceOracleFetchFailed = 722,
    /// The price oracle does not satisfy the contract requirements
    InvalidOracleConfig = 723,
    /// Contract is frozen
    Frozen = 730,
    /// Arithmetic invariant violated
    Overflow = 740,
}
