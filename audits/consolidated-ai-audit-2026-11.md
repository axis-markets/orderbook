# AXIS Orderbook Smart Contract: Consolidated Audit Report

**Date:** 2026-11-13 
**Contract:** `axis-markets-orderbook` v0.7.2 (Soroban, soroban-sdk 29.0.1)
**Code reviewed:** `master` at commit `37fd447`

This is a consolidated report from findings provided by 3 tools: Apex by Cantina, Almanax.ai, and Claude Code (Fable).  

**Sources:**

| Prefix | Source | Findings |
|-|-|-|
| `ORDE-` | Apex | 16 |
| `AX-` | Almanax | 3 |
| `CC-`| Claude Code review | 2 |

Each finding was checked against the code at сщььше `37fd447`.

## Summary

None of the open items puts funds at risk. Two of the three Almanax findings are false positives. They rest on
assumptions about the Soroban host and the build profile that do not hold.

| Status | Count | Findings |
|-|-|-|
| Fixed | 6 | ORDE-1, ORDE-9, ORDE-15, ORDE-19, ORDE-21, CC-1 |
| Acknowledged (by design, documented) | 11 | ORDE-2, ORDE-3, ORDE-4, ORDE-5, ORDE-6, ORDE-11, ORDE-12, ORDE-13, ORDE-17, ORDE-22, ORDE-24 |
| Invalid (false positive) | 2 | AX-1, AX-2 |
| Open | 2 + notes | CC-2 (Low), AX-3 (Informational); notes CC-N1 to CC-N3 |

### All findings

| ID      | Title                                                            | Reported      | Assessed      | Status                    |
|---------|------------------------------------------------------------------|---------------|---------------|---------------------------|
| CC-1    | Anyone can delist a market through a nested `requote`            | Medium        | Medium        | Fixed        |
| ORDE-15 | Lenient settlement accepts malformed token return values         | Medium        | Medium        | Fixed                     |
| ORDE-1  | Future-dated oracle timestamps bypass the 72 h cache limit       | Medium        | Low           | Fixed                     |
| ORDE-21 | `Buy` `Limit` remainder double ceiling over-delivers             | Medium        | Low           | Fixed (bounded to 1%)     |
| ORDE-9  | Crossfill `trade` event misattributes the caller                 | Medium        | Informational | Fixed (redesigned)        |
| ORDE-19 | Swap hop `trade` events misattribute assets and taker            | Informational | Informational | Fixed (redesigned)        |
| CC-2    | Unfilled `FillOrKill` reports `Overflow` instead of `NotFilled`  | Low           | Low           | **Open**                  |
| AX-3    | `intermediate` flag ignored in lenient settlement                | n/a           | Informational | **Open** (latent)         |
| ORDE-3  | `Limit` admission trusts stale `listed` flags                    | Medium        | Low           | Acknowledged              |
| ORDE-5  | `FillOrKill` succeeds after a lower-priced maker is skipped      | Medium        | Low           | Acknowledged              |
| ORDE-6  | Stale signed `update` batch rewrites reused order ids            | Medium        | Low           | Acknowledged (documented) |
| ORDE-11 | Stale signed `trade` recreates a reused order id                 | Medium        | Low           | Acknowledged (documented) |
| ORDE-12 | `requote` can overwrite a fresh cache with an older quote        | Medium        | Low           | Acknowledged              |
| ORDE-13 | An expired oracle feed still counts as listed                    | Medium        | Low           | Acknowledged              |
| ORDE-4  | Crossfill dust rule deletes an executable remainder              | Medium        | Informational | Acknowledged (documented) |
| ORDE-24 | `market` view extends the TTL of a stale record                  | Medium        | Informational | Acknowledged              |
| ORDE-2  | Sell `FillOrKill` succeeds with a raw-unit leftover              | Low           | Informational | Acknowledged (documented) |
| ORDE-17 | Lenient settlement skips makers when the contract's line is full | Low           | Informational | Acknowledged              |
| ORDE-22 | Buy matching skips a partial fill whose full cost exceeds i128   | Low           | Informational | Acknowledged              |
| AX-1    | `__constructor` can be called again to take over the contract    | n/a           | n/a           | Invalid                   |
| AX-2    | Unchecked i128 accumulation wraps settlement totals              | n/a           | n/a           | Invalid                   |

_("Reported" is the source's severity. "Assessed" is the severity after verification against the current code.)_

---

## Open findings

### CC-2: Unfilled `FillOrKill` reports `Overflow` instead of `NotFilled` (Low, open)

**Location:** [src/lib.rs:204](src/lib.rs:204); the same expression is in the `swap` forward pass
at [src/lib.rs:497](src/lib.rs:497).

`FillOrKill` checks whether the unspent leftover could still buy one unit at the highest order price filled:
`mul_div_floor(amount - sold, PRECISION, worst_price) == 0`. When that quotient does not fit in i128, `mul_div_floor`
panics with `Overflow` (740) instead of `NotFilled` (709). For example, a leftover of 10^24 at price 100 does this.

**Impact:** The call reverts either way, so nothing moves. Only the error code is wrong, and a client that re-quotes on
709 handles the trade wrongly. `test_unfilled_fill_or_kill_reports_overflow_for_a_huge_leftover`
([src/tests/lib/requote_context.rs:204](src/tests/lib/requote_context.rs:204)) records the current behavior.

**Recommendation:** A quotient that does not fit i128 means the leftover can certainly buy a unit, so treat it as "not
executed". The private `math::mul_div` already returns `None` in that case. Expose a `try_mul_div_floor` and use
`try_mul_div_floor(..).is_some_and(|q| q == 0)` in both places.

### AX-3: `intermediate` flag ignored in lenient settlement (Informational, open)

**Claim:** In lenient mode, `settle_final` always pays makers with `transfer_from(payer → maker)` and never looks at
`intermediate`. A dispatcher built with `intermediate = true, strict = false` would draw on the payer's allowance
instead of the contract's balance.

**Verification:** The description is accurate. `intermediate` is read only in the strict branch
([src/dispatcher.rs:181-184](src/dispatcher.rs:181)), and `settle_final` always uses `transfer_from`
([src/dispatcher.rs:258](src/dispatcher.rs:258)). No caller combines the two flags this way:

| Call site                                                  | `intermediate` | `strict`             |
|------------------------------------------------------------|----------------|----------------------|
| `trade` ([src/lib.rs:170](src/lib.rs:170))                 | false          | false                |
| `crossfill` ([src/orderbook.rs:162](src/orderbook.rs:162)) | false          | false                |
| `swap` planning ([src/lib.rs:466](src/lib.rs:466))         | true           | true (never settled) |
| `swap` execution ([src/lib.rs:538](src/lib.rs:538))        | `!first`       | true                 |

**Impact:** None today. The latent case also needs a future caller to pass a user account as `payer`, because
`transfer_from` spends only the payer's own allowance to the contract.

**Recommendation:** Make the invalid combination unrepresentable. Replace the two booleans with one mode enum, such as
`Lenient`, `Strict` and `StrictIntermediate`, or assert `!intermediate || strict` in `Dispatcher::new`.

### Notes from the internal review (Informational, open)

- **CC-N1, build reproducibility.** The deployed wasm is built by `scripts/build-nodoc.sh`, which strips doc comments,
  together with the `build.rs` stack flag. A plain `stellar contract build` gives a different hash, so verifying the
  deployed code against its source needs the script. Document the exact build command next to the deployed hash.
- **CC-N2, fee-on-transfer tokens.** The forwarding transfer ([src/dispatcher.rs:217-228](src/dispatcher.rs:217)) sends
  `total_bought` from the contract and assumes the makers delivered that much. With a token that delivers less than it
  reports, the forward either fails the call or, if tokens are stranded in the contract (sent there by mistake), pays
  out of them. Only such tokens are affected. This contradicts the threat model's statement that "no call pays out more
  than it received".
- **CC-N3, non-token addresses.** `subsidize` can open a market whose unlisted side is not a token contract, because
  decimals are read only for listed sides. Orders buying it can be stored but never fill. It costs the listing fee and
  only adds clutter.

---

## Fixed findings

### CC-1: Anyone can delist a market through a nested `requote` (Medium, fixed in `7903b1f`)

**Mechanism:** An attacker called the oracle's `lastprice` with their own account contract as `caller`. The oracle reads
the base asset for free. Its `require_auth` ran the attacker's `__check_auth` while the oracle was on the call stack,
and that code called `axis.requote`. The host refused the nested `expires` calls as re-entry. `try_expires` returned the
refusal as an ordinary error, and `resolve_side` stored both sides as unlisted. This was confirmed by RPC simulation
against Testnet.

**Fix:** `resolve_side` ([src/market.rs:231-238](src/market.rs:231)) now treats only the oracle's own contract error
(`AssetMissing`) as "not listed". Any other failure fails the call with `AssetsNotVerifiedByOracle` (721), so nothing is
written. Regression tests: [src/tests/lib/requote_context.rs:128](src/tests/lib/requote_context.rs:128)
and [:178](src/tests/lib/requote_context.rs:178).

### ORDE-15: Lenient settlement accepts malformed token return values (fixed)

A token whose `transfer_from` or `transfer` returns something other than SEP-41's unit result now fails the call:
`Ok(result) => result.unwrap()` at [src/dispatcher.rs:251-256](src/dispatcher.rs:251)
and [:221-226](src/dispatcher.rs:221).

This differs from the recommendation, which was to skip the maker, and does so on purpose. A token that answers `true`
after moving the tokens would otherwise leave the maker's asset stranded in the contract.
Tests: [src/tests/lib/skip.rs:550-563](src/tests/lib/skip.rs:550).

### ORDE-1: Future-dated oracle timestamps bypass the cache age limit (fixed)

- `fetch_price` ignores a quote dated after the ledger close time ([src/pricing.rs:51-54](src/pricing.rs:51)).
- `age` uses `checked_sub` ([src/pricing.rs:27-29](src/pricing.rs:27)).
- `cached_price` treats a future-dated record as missing ([src/pricing.rs:81-88](src/pricing.rs:81)).

Tests: [src/tests/lib/pricing.rs:404](src/tests/lib/pricing.rs:404), [:438](src/tests/lib/pricing.rs:438), [:452](src/tests/lib/pricing.rs:452).

### ORDE-21: `Buy` remainder double ceiling (fixed, bounded)

The price range is now narrowed to `[100, 10^34]` ([src/math.rs:8-10](src/math.rs:8)). The two bounds are each other's
inverse, so a price and its rounded inverse move in steps of at most 1%. The PoC price `PRECISION² − 1` is now rejected
with `InvalidPrice`.

The remainder still rounds both factors up, so its fill can deliver up to about 1% more than the target, plus amount
rounding. The trader never pays more than their limit. This is documented in the README "Rounding" section.
Test: [src/tests/lib/buy.rs:57](src/tests/lib/buy.rs:57).

### ORDE-9 and ORDE-19: `trade` event attribution in `crossfill` and `swap` (fixed by redesign)

**What changed:** `taker` is now the account each fill is made for:

- In `crossfill`, the caller on every maker fill.
- In `swap`, the trader on every hop ([src/lib.rs:538-547](src/lib.rs:538)).

The contract never appears in an event. `crossfill` also reports the taker order's own fill, in the reverse direction,
with the caller as taker and the owner as maker.

**Why that resolves both findings:** The `trade` events of a call, summed per account, now equal each account's net
balance change. In the ORDE-9 PoC, the caller's two events net to +240 EUR and 0 USD, which is exactly the surplus
received. The aggregate `swap` event still carries the route totals.

**Tests and documentation:** The rule is documented in README "Events → trade". `assert_trades_match_balances` checks it
in `crossfill.rs`, `swap.rs`, `events.rs` and `custom_token.rs`. The axis-indexer was updated to match.

---

## Acknowledged findings (by design or out of scope)

These were reviewed and ruled on after the October 2026 report. The code around them is unchanged at `37fd447`, so the
rulings still apply.

| ID              | Still present?                                                                                                                    | Why accepted                                                                                                                                                                                                                                                                                                                    |
|-----------------|-----------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| ORDE-3          | Yes, `load_listed_market` uses the stored flags ([src/market.rs:279](src/market.rs:279))                                          | Listing is refreshed lazily by design; anyone can call `requote`. Until someone does, `Limit` admission and the minimum-size check use the old flags; with a non-zero floor the stale price must also still be within 72 h.                                                                                                     |
| ORDE-13         | Yes, any successful `expires` counts as listed ([src/market.rs:232](src/market.rs:232))                                           | `listed` means the oracle supports the asset, not that access is paid up; `track` revives an expired feed. Probing access instead deadlocks re-funding. With a non-zero floor, no usable price means 722.                                                                                                                       |
| ORDE-24         | Yes ([src/market.rs:207-214](src/market.rs:207))                                                                                  | The view keeping a market alive is intended. The stale flags are the same issue as ORDE-3.                                                                                                                                                                                                                                      |
| ORDE-12         | Yes, no timestamp ordering in `fetch_price` ([src/pricing.rs:55-72](src/pricing.rs:55))                                           | Reflector's `lastprice` is monotonic, so an older quote can only arrive after the admin calls `set_oracle` to an oracle with the same decimals. Availability only (722 until fresh data).                                                                                                                                       |
| ORDE-6, ORDE-11 | Yes, ids are `(owner, nonce)` and reusable after expiry or removal ([src/order.rs:59](src/order.rs:59), [:125](src/order.rs:125)) | Needs the owner to reuse a nonce while still holding an unsubmitted signed transaction. That is the Soroban authorization model and the client's choice of nonce. README "Order ids" discourages reuse; clients derive nonces from the clock plus random bits. ORDE-11 recreates exactly what the owner signed.                 |
| ORDE-2          | Yes ([src/lib.rs:200-207](src/lib.rs:200))                                                                                        | Documented FoK leftover rule (README "Rounding"): the leftover cannot buy one unit at the highest price filled and is less than the amount sold. Tests in `dust.rs`.                                                                                                                                                            |
| ORDE-5          | Yes: after a skip, the check uses the highest *settled* price ([src/lib.rs:219](src/lib.rs:219))                                  | Falls under the same documented FoK rule, so the bound above holds. Optional hardening: fail `FillOrKill` whenever an admitted fill is skipped at settlement.                                                                                                                                                                   |
| ORDE-4          | Yes ([src/orderbook.rs:199-202](src/orderbook.rs:199), [src/dispatcher.rs:194-197](src/dispatcher.rs:194))                        | Documented dust rule (README "Rounding"). The removed remainder is worth less than one base unit of the counter asset at the order's own price, and the owner keeps the tokens. The narrowed price range rejects the PoC's price of 1, but the mechanism remains at, for example, price 100 with a remainder under 10^16 units. |
| ORDE-17         | Yes, `Err(_) => return false` cannot tell maker-side from receiver-side failures ([src/dispatcher.rs:254](src/dispatcher.rs:254)) | SAC balances held by contract addresses have no trustline limit. Only custom tokens with per-holder caps are affected, and the result is a partial fill with a false `skip`, with no loss.                                                                                                                                      |
| ORDE-22         | Yes ([src/orderbook.rs:114-124](src/orderbook.rs:114))                                                                            | Needs amounts close to i128::MAX whose cost exceeds i128. The taker can retry with a smaller amount.                                                                                                                                                                                                                            |

---

## Invalid findings

### AX-1: Public constructor allows reinitialization takeover (invalid)

**Claim:** `__constructor` can be called after deployment to overwrite `Config`, setting the safety admin and oracle to
attacker-chosen addresses.

**Why it is invalid:** The Soroban host refuses a direct call to any function whose name starts with `__` unless the
host itself makes it. In soroban-env-host 29.0.1 this is `RESERVED_CONTRACT_FN_PREFIX` in `src/host/frame.rs:36-39` and
the check at `:1136-1147`, which fails with "can't invoke a reserved function directly". The constructor runs once,
inside contract creation (CAP-0058).

**Probe:** A temporary test called `__constructor` on a deployed instance with attacker addresses. The call failed with
`Error(Context, InvalidAction)`, and `config()` still returned the original safety admin and oracle.

### AX-2: Unchecked i128 accumulation can wrap settlement totals (invalid)

**Claim:** `planned()` and `settle()` sum amounts with unchecked arithmetic, which wraps silently in release builds.

**Why it is invalid:** The premise is wrong for this crate. [Cargo.toml:12](Cargo.toml:12) sets `overflow-checks = true`
in `[profile.release]`, the profile `stellar contract build` uses, and the dev and test profiles check overflow by
default. An overflow traps and reverts the whole transaction, so no transfer, event or skipped forward can see a wrapped
value.

The sums are also bounded in practice:

- A `Sell` sums `sold` to at most `amount`, and a `Buy` sums `bought` to at most `amount`.
- Each maker's `maker_sends` is at most that maker's backing.

Overflow would need a custom token whose balances add up past i128::MAX, or a `Buy` cost past i128 that no taker can
pay. Both revert.
