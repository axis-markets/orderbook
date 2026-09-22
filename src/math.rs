//! Fixed-point arithmetic.
use crate::errors::OrderbookError;
use soroban_sdk::{Env, U256};

/// Fixed-point price precision (18 decimals)
pub const PRECISION: i128 = 10i128.pow(18);
/// Lowest accepted order price
pub const MIN_PRICE: i128 = 1;
/// Highest accepted order price, the inverse of `MIN_PRICE`
pub const MAX_PRICE: i128 = PRECISION * PRECISION;

/// Reject a price outside the accepted range
#[inline]
pub(crate) fn check_price(e: &Env, price: i128) {
    if !(MIN_PRICE..=MAX_PRICE).contains(&price) {
        e.panic_with_error(OrderbookError::InvalidPrice);
    }
}

/// `a * b / d` with a 256-bit fallback for products beyond i128.
/// Returns the floored quotient and whether a remainder was dropped, `None` when the quotient does not fit i128
fn mul_div(e: &Env, a: i128, b: i128, d: i128) -> Option<(i128, bool)> {
    check_mul_div_operands(e, a, b, d);
    if let Some(product) = a.checked_mul(b) {
        return Some((product / d, product % d != 0));
    }
    let product = U256::from_u128(e, a as u128).mul(&U256::from_u128(e, b as u128));
    let divisor = U256::from_u128(e, d as u128);
    let quotient = product
        .div(&divisor)
        .to_u128()
        .filter(|q| *q <= i128::MAX as u128)? as i128;
    //the remainder is below `d`, so it always fits
    let remainder = product.rem_euclid(&divisor).to_u128().unwrap_or(1);
    Some((quotient, remainder != 0))
}

/// `floor(a * b / d)` for non-negative operands
#[inline]
pub(crate) fn mul_div_floor(e: &Env, a: i128, b: i128, d: i128) -> i128 {
    match mul_div(e, a, b, d) {
        Some((quotient, _)) => quotient,
        None => e.panic_with_error(OrderbookError::Overflow),
    }
}

/// `ceil(a * b / d)` for non-negative operands, `None` when the result does not fit i128
#[inline]
pub(crate) fn try_mul_div_ceil(e: &Env, a: i128, b: i128, d: i128) -> Option<i128> {
    let (quotient, inexact) = mul_div(e, a, b, d)?;
    if inexact {
        quotient.checked_add(1)
    } else {
        Some(quotient)
    }
}

/// `ceil(a * b / d)` for non-negative operands
#[inline]
pub(crate) fn mul_div_ceil(e: &Env, a: i128, b: i128, d: i128) -> i128 {
    match try_mul_div_ceil(e, a, b, d) {
        Some(quotient) => quotient,
        None => e.panic_with_error(OrderbookError::Overflow),
    }
}

/// Price seen from the other side of the market, rounded down
#[inline]
pub(crate) fn invert_price_floor(e: &Env, price: i128) -> i128 {
    mul_div_floor(e, PRECISION, PRECISION, price)
}

/// Price seen from the other side of the market, rounded up
#[inline]
pub(crate) fn invert_price_ceil(e: &Env, price: i128) -> i128 {
    mul_div_ceil(e, PRECISION, PRECISION, price)
}

/// Whether `amount` at `price` is worth less than one unit of the counter asset.
#[inline]
pub(crate) fn is_dust(e: &Env, amount: i128, price: i128) -> bool {
    mul_div_floor(e, amount, price, PRECISION) == 0
}

#[inline]
fn check_mul_div_operands(e: &Env, a: i128, b: i128, d: i128) {
    if a < 0 || b < 0 || d <= 0 {
        e.panic_with_error(OrderbookError::Overflow);
    }
}
