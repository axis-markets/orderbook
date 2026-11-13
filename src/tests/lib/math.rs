//! Fixed-point arithmetic with the 256-bit fallback.
use crate::math::{
    invert_price_ceil, invert_price_floor, is_dust, mul_div_ceil, mul_div_floor, try_mul_div_floor,
    MAX_PRICE, MIN_PRICE, PRECISION,
};
use soroban_sdk::Env;

const BIG: i128 = 10i128.pow(30);

#[test]
fn test_mul_div_fast_path() {
    let e = Env::default();
    assert_eq!(mul_div_floor(&e, 7, 1, 2), 3);
    assert_eq!(mul_div_ceil(&e, 7, 1, 2), 4);
    assert_eq!(mul_div_floor(&e, 8, 1, 2), 4);
    assert_eq!(mul_div_ceil(&e, 8, 1, 2), 4);
    assert_eq!(mul_div_floor(&e, 0, 5, 3), 0);
    assert_eq!(mul_div_ceil(&e, 0, 5, 3), 0);
}

#[test]
fn test_mul_div_wide_fallback() {
    let e = Env::default();
    // 10^60 does not fit i128, the quotient does
    assert_eq!(mul_div_floor(&e, BIG, BIG, BIG), BIG);
    assert_eq!(mul_div_ceil(&e, BIG, BIG, BIG), BIG);
    assert_eq!(mul_div_floor(&e, BIG, BIG, 3 * BIG), BIG / 3);
    assert_eq!(mul_div_ceil(&e, BIG, BIG, 3 * BIG), BIG / 3 + 1);
    // the largest price times the largest amount the fast path cannot take
    assert_eq!(
        mul_div_floor(&e, i128::MAX, MAX_PRICE, MAX_PRICE),
        i128::MAX
    );
}

#[test]
#[should_panic(expected = "#740")]
fn test_mul_div_overflowing_result() {
    let e = Env::default();
    mul_div_floor(&e, BIG, BIG, 1);
}

#[test]
fn test_try_mul_div_floor_answers_none_beyond_i128() {
    let e = Env::default();
    assert_eq!(try_mul_div_floor(&e, 7, 1, 2), Some(3));
    assert_eq!(try_mul_div_floor(&e, 0, 5, 3), Some(0));
    assert_eq!(try_mul_div_floor(&e, BIG, BIG, BIG), Some(BIG));
    // the largest quotient that fits, and the first that does not
    assert_eq!(try_mul_div_floor(&e, i128::MAX, 2, 2), Some(i128::MAX));
    assert_eq!(try_mul_div_floor(&e, i128::MAX, 2, 1), None);
    assert_eq!(try_mul_div_floor(&e, BIG, BIG, 1), None);
}

#[test]
#[should_panic(expected = "#740")]
fn test_try_mul_div_floor_rejects_negative_operands() {
    let e = Env::default();
    try_mul_div_floor(&e, -1, 1, 1);
}

#[test]
#[should_panic(expected = "#740")]
fn test_mul_div_rejects_negative_operands() {
    let e = Env::default();
    mul_div_floor(&e, -1, 1, 1);
}

#[test]
#[should_panic(expected = "#740")]
fn test_mul_div_rejects_zero_divisor() {
    let e = Env::default();
    mul_div_ceil(&e, 1, 1, 0);
}

#[test]
fn test_invert_price() {
    let e = Env::default();
    assert_eq!(invert_price_floor(&e, PRECISION), PRECISION);
    assert_eq!(invert_price_ceil(&e, PRECISION), PRECISION);
    assert_eq!(invert_price_floor(&e, 2 * PRECISION), PRECISION / 2);
    assert_eq!(
        invert_price_floor(&e, 3 * PRECISION),
        333_333_333_333_333_333
    );
    assert_eq!(
        invert_price_ceil(&e, 3 * PRECISION),
        333_333_333_333_333_334
    );
    // the extremes map onto each other
    assert_eq!(invert_price_floor(&e, MIN_PRICE), MAX_PRICE);
    assert_eq!(invert_price_ceil(&e, MIN_PRICE), MAX_PRICE);
    assert_eq!(invert_price_floor(&e, MAX_PRICE), MIN_PRICE);
    assert_eq!(invert_price_ceil(&e, MAX_PRICE), MIN_PRICE);
    // the coarsest inverse rounds up by a single step, 1% of the lowest price
    assert_eq!(invert_price_ceil(&e, MAX_PRICE - 1), MIN_PRICE + 1);
}

#[test]
fn test_dust() {
    let e = Env::default();
    assert!(is_dust(&e, 1, PRECISION - 1));
    assert!(!is_dust(&e, 1, PRECISION));
    assert!(is_dust(&e, 999, PRECISION / 1000));
    assert!(!is_dust(&e, 1000, PRECISION / 1000));
    assert!(!is_dust(&e, 1, MAX_PRICE));
}
