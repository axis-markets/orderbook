use soroban_sdk::contracterror;

/// Standard contract errors
#[contracterror]
#[repr(i16)]
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderbookError {
    NotAuthorized = 701,
    InsufficientBalance = 702,
    OrderNotFound = 703,
    Overflow = 704,
    InvalidMatch = 705,
    InvalidPrice = 706,
}
