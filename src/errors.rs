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
    /// Provided order does not match selling/buying asset
    InvalidMatch = 704,
    /// Price is out of range
    InvalidPrice = 705,
    /// The trade amount is invalid (negative)
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
    /// Order value is below the minimum order size
    OrderSizeTooSmall = 720,
    /// The market does not exist, or none of its assets is listed on the price oracle
    AssetsNotVerifiedByOracle = 721,
    /// No usable price is cached for the asset: none was fetched by `requote` or `subsidize`
    /// under the current oracle, or the cached one is older than 72 hours
    AssetPriceOracleFetchFailed = 722,
    /// The price oracle does not satisfy the contract requirements
    InvalidOracleConfig = 723,
    /// Contract is frozen
    Frozen = 730,
    /// Arithmetic invariant violated: a `mul_div` quotient that does not fit i128, a negative
    /// operand or zero divisor, or a negative crossfill surplus. Plain arithmetic overflow is not
    /// reported with this code: `overflow-checks` makes it trap
    Overflow = 740,
}
