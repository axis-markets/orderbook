//! Fixed-point arithmetic. Plain arithmetic elsewhere relies on `overflow-checks = true` and
//! traps on overflow. `mul_div` keeps a U256 fallback on purpose: the 18-decimal fixed point
//! uses most of i128, so `amount * price` overflows for realistic inputs (a few hundred units of
//! an 18-decimal token, or any large order at a high price) while the quotient fits. Only a
//! quotient that does not fit i128 is an error (`Overflow`).
use crate::errors::OrderbookError;
use soroban_sdk::{Env, U256};

/// Fixed-point price precision (18 decimals)
pub const PRECISION: i128 = 10i128.pow(18);
/// Lowest accepted order price
pub const MIN_PRICE: i128 = 1;
/// Highest accepted order price, the inverse of `MIN_PRICE`
pub const MAX_PRICE: i128 = PRECISION * PRECISION;

/// Reject a price outside the accepted range
pub(crate) fn check_price(e: &Env, price: i128) {
    if !(MIN_PRICE..=MAX_PRICE).contains(&price) {
        e.panic_with_error(OrderbookError::InvalidPrice);
    }
}

fn check_mul_div_operands(e: &Env, a: i128, b: i128, d: i128) {
    if a < 0 || b < 0 || d <= 0 {
        e.panic_with_error(OrderbookError::Overflow);
    }
}

/// `a * b / d` in 256 bits
/// return floored quotient and whether a remainder was dropped
fn u256_mul_div(e: &Env, a: i128, b: i128, d: i128) -> (i128, bool) {
    let product = U256::from_u128(e, a as u128).mul(&U256::from_u128(e, b as u128));
    let divisor = U256::from_u128(e, d as u128);
    let quotient = match product.div(&divisor).to_u128() {
        Some(q) if q <= i128::MAX as u128 => q as i128,
        _ => e.panic_with_error(OrderbookError::Overflow),
    };
    //the remainder is below `d`, so it always fits
    let remainder = product.rem_euclid(&divisor).to_u128().unwrap_or(1);
    (quotient, remainder != 0)
}

/// `floor(a * b / d)` for non-negative operands
pub(crate) fn mul_div_floor(e: &Env, a: i128, b: i128, d: i128) -> i128 {
    check_mul_div_operands(e, a, b, d);
    match a.checked_mul(b) {
        Some(product) => product / d,
        None => u256_mul_div(e, a, b, d).0,
    }
}

/// `ceil(a * b / d)` for non-negative operands
pub(crate) fn mul_div_ceil(e: &Env, a: i128, b: i128, d: i128) -> i128 {
    check_mul_div_operands(e, a, b, d);
    let (quotient, inexact) = match a.checked_mul(b) {
        Some(product) => (product / d, product % d != 0),
        None => u256_mul_div(e, a, b, d),
    };
    if !inexact {
        return quotient;
    }
    //overflow checks trap the single quotient that cannot take the rounding step
    quotient + 1
}

/// Price seen from the other side of the market, rounded down
pub(crate) fn invert_price_floor(e: &Env, price: i128) -> i128 {
    mul_div_floor(e, PRECISION, PRECISION, price)
}

/// Price seen from the other side of the market, rounded up
pub(crate) fn invert_price_ceil(e: &Env, price: i128) -> i128 {
    mul_div_ceil(e, PRECISION, PRECISION, price)
}

/// Whether `amount` at `price` is worth less than one unit of the counter asset.
pub(crate) fn is_dust(e: &Env, amount: i128, price: i128) -> bool {
    mul_div_floor(e, amount, price, PRECISION) == 0
}
