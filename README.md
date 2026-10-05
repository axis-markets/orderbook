# `@axis-markets/orderbook`

> Stellar smart contract for AXIS limit orderbook DEX.

The contract holds no user funds between calls. Makers and takers grant the contract a standing token allowance, and
every fill is settled with `transfer_from`, the contract acting as the spender: the taker pays each maker directly,
while the makers' assets pass through the contract on their way to the taker within the call. An order is backed by its
owner's balance and allowance. A fill the maker cannot back, or whose payment the maker cannot receive, is skipped: the
order is left unchanged and a `skip` event tells indexers which maker failed to settle.

## Interface

`fn __constructor(safety_admin: Address, oracle: Address, min_trade_size: i128)`
Create new CLOB contract with a given minimum trade size and oracle.

Arguments

- `safety_admin` - Account allowed to freeze the contract and adjust the oracle
- `oracle` - Price oracle contract address
- `min_trade_size` - Minimum trade value in USD with 7 decimals, zero disables the limit

Panics

- If the minimum trade size is negative (`InvalidAmount`)
- If the oracle is invalid (`InvalidOracleConfig`)

---

`fn config() -> Config`
Fetch contract configuration parameters.

Returns

- Current contract configuration

---

`fn frozen() -> bool`
Check whether the contract is frozen.

Returns

- `true` if contract is frozen (trading and order management operations blocked)

---

`fn market(selling: Address, buying: Address) -> Option<Market>`
Fetch market for the asset pair.

Arguments

- `selling` - First market asset
- `buying` - Second market asset

Returns

- Market record if the market exists

---

`fn order(id: u128) -> Option<Order>`
Fetch existing order.

Arguments

- `id` - ID of the order to fetch

Returns

- Order fetched from the storage, `None` if it does not exist or has expired

---

`fn trade(direction: TradeDirection, kind: OrderKind, trader: Address, amount: i128, selling: Address, buying: Address, price: i128, orders: Vec<u128>, nonce: u64, expires: u64, approve: Option<Approval>) -> (i128, i128, Option<u128>)`
Trade with DEX and create a limit order if the quote was not executed in full.

Arguments

- `direction` - Trade direction: `Sell` or `Buy`
- `kind` - Order type (`Limit`, `Fill`, `FillOrKill`)
- `trader` - Trader address
- `amount` - Amount of `selling` tokens to send for `Sell` orders or target amount of `buying` tokens to acquire for
  `Buy` orders
- `selling` - Token address sent by trader
- `buying` - Token address received by trader
- `price` - Price limit: minimum `buying` per 1 `selling` for `Sell` orders or maximum `selling` per 1 `buying` for
  `Buy` orders
- `orders` - Optional list of order IDs to match before creating the order on-chain
- `nonce` - Client-chosen nonce for the id of the order created for the remainder
- `expires` - Expiration timestamp of the order created for the remainder (0 = no expiration), checked for `Limit`
  trades only
- `approve` - Optional allowance granted to the contract before trading, normally on `selling`

Returns

- Amount of sold tokens
- Amount of bought tokens
- ID of the newly created order, if any

Panics

- If the contract is frozen
- If `amount` is invalid, `price` is out of range, or `selling` equals `buying`
- If the trader cannot pay for the fills, or cannot receive `buying`
- If the contract cannot hold `buying` on its way to the trader (`IntermediaryCannotReceive`)
- If a `FillOrKill` trade cannot be executed in full (`NotFilled`)
- If the remainder is not backed by the trader's balance and allowance
- If an order with the same id is already on the book (`OrderExists`)
- If a `Limit` trade `expires` is set in the past (`InvalidExpiration`)
- If a `Limit` trade's market does not exist
- If a `Limit` trade sells less than the minimum order value (`OrderSizeTooSmall`)
- If neither of the order assets is quoted by the oracle (`AssetsNotVerifiedByOracle`)
- If no usable price is cached for the asset it is valued on (`AssetPriceOracleFetchFailed`)

---

`fn update(trader: Address, updates: Vec<OrderUpdate>, approvals: Vec<Approval>) -> Vec<u128>`
Update the amount, price, or expiration of several orders in place, or remove them. Orders that no longer exist are
skipped; expired orders revived with the new expiration. A zero amount removes the order, expired ones included. An
updated order's entry lifetime is extended to cover its expiration +1 day. Approvals and removals work in a frozen
contract and on a market without quoted assets.

Arguments

- `trader` - Orders owner
- `updates` - New amount, price, and expiration per order
- `approvals` - Allowances granted to the contract before the backing check; an asset without an approval keeps its
  current allowance

Returns

- IDs of the orders updated or removed

Panics

- If `trader` does not own an updated order (`NotAuthorized`)
- If an `amount` is negative or `price` is out of range
- If an `expires` is set in the past (`InvalidExpiration`)
- If the new amounts are not backed by the trader's balance and allowance
- If an order is below the minimum value
- If the contract is frozen and an update changes an order (`Frozen`)
- If an update changes an order whose market has no quoted asset (`AssetsNotVerifiedByOracle`)
- If no listed market asset has a usable cached price (`AssetPriceOracleFetchFailed`)

---

`fn crossfill(trader: Address, taker_order_id: u128, orders: Vec<u128>) -> (i128, i128, i128)`
Fill an existing order against matching orders from the orderbook. Profits from inefficiencies go to the trader.

Arguments

- `trader` - Trader address
- `taker_order_id` - ID of the order that serves as a taker
- `orders` - List of order IDs to match against

Returns

- Amount the taker order sold
- Amount the makers delivered
- Surplus paid to `trader`

Panics

- If the contract is frozen
- If the taker order does not exist or has expired (`OrderNotFound`)
- If the contract cannot hold the bought asset (`IntermediaryCannotReceive`)
- If `trader` cannot receive the bought asset (`CannotReceive`)

---

`fn swap(direction: TradeDirection, trader: Address, selling: Address, selling_amount: i128, buying_amount: i128, path: Vec<TradeStep>, approve: Option<Approval>) -> (i128, i128)`
Swap tokens across several markets. The contract holds the intermediate hop proceeds only within the call.

Arguments

- `direction` - Trade direction: `Sell` or `Buy`
- `trader` - Trader address
- `selling` - Token address sent by the trader
- `selling_amount` - Amount of selling tokens to send (`Sell`) or the maximum to spend (`Buy`)
- `buying_amount` - Minimum amount of buying tokens to receive (`Sell`) or the exact amount (`Buy`)
- `path` - Ordered list of the trade route steps
- `approve` - Optional allowance granted to the contract before trading, normally on `selling`

Returns

- Amount of sold tokens
- Amount of bought tokens

Panics

- If the contract is frozen
- If `path` is empty or an amount is not positive
- If the contract cannot hold an asset of the path (`IntermediaryCannotReceive`)
- If the route cannot satisfy the selling/buying amount (`NotFilled`)
- If the trader cannot pay for the fills, or cannot receive `buying`

---

`fn requote(selling: Address, buying: Address) -> Option<Market>`
Re-check both market assets against the price oracle and cache oracle prices. A cached price is valid for up to 72
hours. A market without quoted assets stops accepting new limit orders; its outstanding orders stay cancellable and
fillable. The record and the cached prices are rewritten only when they change.

Arguments

- `selling` - First market asset
- `buying` - Second market asset

Returns

- Updated market record, `None` if the market does not exist

Panics

- If the contract is frozen

---

`fn subsidize(sponsor: Address, selling: Address, buying: Address, amount: i128) -> Vec<u64>`
Extend oracle price feeds access for a market, creating the market if it does not exist yet.

Arguments

- `sponsor` - Address paying for the oracle feeds
- `selling` - First market asset
- `buying` - Second market asset
- `amount` - Amount of fee tokens to burn

Returns

- New access expiration UNIX timestamps (in seconds) per oracle-listed asset

Panics

- If `amount` is invalid
- If `selling` equals `buying` (`InvalidMatch`)
- If the market does not exist and the amount is below the market listing fee (`InvalidAmount`)
- If neither market asset is quoted by the oracle (`AssetsNotVerifiedByOracle`)
- If the contract is frozen

---

`fn freeze(blocked: bool)`
Toggle freezing the contract. A frozen DEX blocks every trading and order management call.

Arguments

- `blocked` - `true` blocks trading, `false` resumes normal operation

Panics

- If the call is not authorized by the safety admin

---

`fn delegate(new_safety_admin: Address)`
Transfer the safety admin role over to another account.

Arguments

- `new_safety_admin` - Account that takes over the safety admin role

Panics

- If the call is not authorized by the current safety admin

---

`fn set_oracle(oracle: Address)`
Point the contract at another price oracle.

Arguments

- `oracle` - Reflector Beam price oracle contract address

Panics

- If the call is not authorized by the safety admin
- If the oracle quotes prices with too many decimals, or charges no daily fee (`InvalidOracleConfig`)

---

`fn set_floor(minimum: i128)`
Set the protocol minimum trade size.

Arguments

- `minimum` - Minimum trade value in USD with 7 decimals (1 USD = 10_000_000)

Panics

- If the call is not authorized by the safety admin
- If `minimum` is negative

---

`fn set_listing_min_days(days: u32)`
Set the days of price feeds a new market must provision.

Arguments

- `days` - Days of price feeds the market listing fee buys

Panics

- If the call is not authorized by the safety admin
- If `days` exceeds 255 (`InvalidAmount`)
- If the oracle charges no daily fee (`InvalidOracleConfig`)

---

`fn set_ledger_time(ledger_time: u32)`
Set the average expected ledger close time.

Arguments

- `ledger_time` - Ledger close time in seconds

Panics

- If the call is not authorized by the safety admin
- If `ledger_time` is zero or exceeds 20 seconds (`InvalidAmount`)


## Allowances

Every asset a trader sells through the contract needs a standing allowance:
`approve(from = trader, spender = AXIS contract, amount, live_until_ledger)` on the token. The allowance is spent by
fills of the trader's own orders (at their price) and by the trader's own trades; `update` never spends it. An expired
allowance reads as zero and must be granted again.

`Approval { asset, amount, live_until }` passed to `trade` or `swap` (normally on `selling`), or in the `approvals`
list of `update`, performs the `approve` on `asset` inside the call as a sub-invocation the trader signs. `update`
leaves the allowance on an asset without an approval unchanged. The amount is absolute, so the signed authorization does
not depend on the ledger state.

The book does not guarantee that an open order is backed: the owner can spend the balance or revoke the allowance at any
time. A listed order fills only when the maker's backing left covers that fill (one budget shared by the maker's orders
in list order) and the maker can receive the taker's asset; otherwise it is skipped, left unchanged, and a `skip`
event names it. The same happens when the maker's transfer fails despite the backing (classic liabilities, reserve, a
deauthorized trustline). The payment to the maker is never probed: when it fails, whether the taker cannot pay or the
maker cannot be credited although authorized (e.g. a classic trustline with limit, which `authorized` does not
show), the call fails with the token's own error, nothing moves and no maker is flagged, the caller leaves that maker
out and retries. Telling the two apart would take a probe transfer that spends the payer's allowance, and in
`crossfill` the payer is the order owner, who did not sign the call. A taker who cannot receive the bought asset fails
the trade with `CannotReceive`. Matching never removes, trims or caps an order. Routers should compute the effective
depth per maker and asset as `min(balance, allowance)` shared across that maker's orders, treat the allowance's
`live_until_ledger` as an expiry, check that the maker's line for the taker's asset has room for the payment, and use
`skip` events to stop proposing orders of makers that fail to settle.

## Rounding

Every fill rounds in the maker's favor: the taker receives a rounded-down amount and pays a rounded-up one, so a maker
is never paid below the order price. The taker's `price` limit is compared with the order prices, not with the rate
each fill ends up at, so rounding can take a fill past the limit by less than one base unit of the asset the taker
pays. For example, a maker sells 2 EUR at 0.6 USD/EUR: the whole order costs 1.2 USD, rounded up to 2. A taker selling
USD for at least 1.6 EUR per USD accepts the order (0.6 is below 1 / 1.6) and gets 2 EUR for 2 USD, 1 EUR per USD.

The allowance is under one base unit of the paid asset per fill, so under 20 units per trade at the fill cap. That is
negligible for 7-decimal Stellar assets, but a unit of a token with few decimals can carry real value. Fills are
deterministic, so routers trading such tokens should compute them before listing an order and leave out orders whose
rounding exceeds what the taker accepts: selling into an order buys `floor(amount × 10^18 / order.price)` for
`ceil(bought × order.price / 10^18)`, or takes the whole order for `ceil(order.amount × order.price / 10^18)` when the
amount covers it; buying pays `ceil(bought × order.price / 10^18)`. `swap` also bounds the whole route with
`selling_amount` and `buying_amount`.

The limit is not enforced on the rounded amounts on purpose: that would refuse a fill at the limit price whenever the
amounts do not divide exactly, and the 18-decimal inverse of a price is itself rounded, so even exact fills against an
order priced at the inverted limit would fall a fraction of a unit short of it.

## Order ids

An order ID is deterministically derived from the client-provided `u64` nonce:

```
id = u128::from_be_bytes(sha256(xdr([owner, nonce]))[0..16])
```

Hashed bytes are the XDR of the `ScVal` vector `[owner: Address, nonce: u64]`. The nonce must be unique among the
owner's live orders (a duplicate fails with `OrderExists`); once the order is gone or has expired the nonce may be
reused, and a new order overwrites the expired entry (its lifetime is extended as for an `update`). Clients typically
derive it from the clock plus random bits. The id is known before the transaction is simulated, so the order entry can
be declared in the footprint.

## Order expiration

An order with a non-zero `expires` stops being live once the ledger timestamp reaches it (`expires <= now`). From then
on the contract treats it as gone: matching skips it (`trade`, `swap`, `crossfill` makers), `crossfill` fails on it as
the taker order (`OrderNotFound`), `order` returns `None`, and its id is free for a new order. The entry itself stays in
storage until `update` removes it (`amount = 0`, owner only), a new order under the same id overwrites it, or its TTL
runs out. `update` also revives an expired order: it rewrites the entry with the new amount, price and expiration, which
must be `0` or in the future like any update, and emits `mod`; indexers should therefore keep an expired order's record
until it is removed rather than forget it at expiry. The expiration is set by `trade` for the stored remainder and
changed by `update` (`0` lifts it); both reject a timestamp that is not in the future (`InvalidExpiration`). The `new`
and `mod` events carry it.

## Order updates

`update` sets the amount, price and expiration of an order to the values given. The amount is absolute, not a change
from the current amount, so a fill that lands between signing and execution is not taken into account: a maker who
shrinks an order from 1000 to 800 while a 500 fill is in flight ends up with 800 on the book after the fill, selling
1300 in total instead of 800 (at their price, and backed by their balance and allowance). To cut exposure whatever
fills are in flight, remove the order (`amount = 0`) or lower the allowance in the same call (`approvals`), which caps
what all the maker's orders on that asset can still sell; read the order again before resizing it.

## Order storage format

The `order` view returns the full record:

```rust
struct Order {
    /// Unique order identifier, derived from the owner and the client nonce
    pub id: u128,
    /// Selling token address
    pub selling: Address,
    /// Buying token address
    pub buying: Address,
    /// Amount left to sell
    pub amount: i128,
    /// Maker address
    pub owner: Address,
    /// Order price (`buying` per 1 `selling`, 18 decimals)
    pub price: i128,
    /// Expiration timestamp (0 = no expiration)
    pub expires: u64,
}
```

The ledger entry (a persistent entry keyed by the raw `u128` id) stores it positionally, without the id, to keep the
entry small: a `Vec` of `[owner, selling, buying, amount, price]`, with `expires` appended only when it is set. An order
without expiration takes 172-176 bytes of value (about 250 bytes of ledger entry).

Orders are stored sell-equivalent: a `Buy` remainder stored as "sell `ceil(amount × price)` of the selling asset at the
rounded-up inverse price". A new order entry gets the network minimum lifetime (about 120 days). Every `update` that
changes an order, and a new order written over an expired entry under the same id, extends the entry to cover the
expiration plus a day, or to 120 days for an order without expiration (never shortening it). Fills and removals do not
extend it. Whoever touches an archived order pays for its restoration.

## Argument types

```rust
struct TradeStep {
    /// Asset to buy at this step
    pub asset: Address,
    /// Maker order ids to match
    pub orders: Vec<u128>,
}

struct Approval {
    /// Token the allowance is granted on
    pub asset: Address,
    /// Absolute allowance amount
    pub amount: i128,
    /// Ledger sequence the allowance lives until
    pub live_until: u32,
}

struct OrderUpdate {
    /// Order id
    pub id: u128,
    /// New amount to sell, 0 removes the order
    pub amount: i128,
    /// New order price (ignored when the order is removed)
    pub price: i128,
    /// New expiration timestamp, 0 = no expiration (ignored when the order is removed)
    pub expires: u64,
}
```

## Market storage format

```rust
struct MarketSide {
    /// Token contract address
    pub asset: Address,
    /// Whether the asset is quoted by the oracle (with token decimals the valuation can handle)
    pub listed: bool,
    /// Token decimals (fetched only for listed assets)
    pub decimals: u32,
}

struct Market {
    /// Base asset, the first of the pair in canonical order
    pub base: MarketSide,
    /// Quote asset, the second of the pair in canonical order
    pub quote: MarketSide,
    /// Creation timestamp
    pub created: u64,
}
```

The record changes only when a side's listing or decimals change. Every check against the oracle (`requote`,
`subsidize`, market creation) emits a `refresh` event, which marks the time of the last verification. An asset whose
token decimals and oracle decimals add up to more than 37 cannot be valued, so it is recorded as unlisted (with zero
decimals) even when the oracle quotes it; after `set_oracle` to an oracle with more decimals, the next check of a
market delists such an asset instead of failing.

Reading a market (`requote`, `subsidize`, the `market` view) extends its entry to 120 days once less than 30 are left.
Writing an order to it (a `Limit` trade, an `update` that changes an order) extends it to 121 days once less than 120
are left, so the market outlives every order written to it; a busy market pays for one extension a day.

## Standard errors

```rust
enum OrderbookError {
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
```

## Configuration

```rust
struct Config {
    /// Account allowed to freeze the contract and change the configuration
    pub safety_admin: Address,
    /// Price oracle contract address
    pub oracle: Address,
    /// Days of price feeds a new market must buy, zero opens markets without a fee
    pub listing_min_days: u32,
    /// Amount of oracle fee tokens paid by the market creator to provision the oracle price
    /// feeds for the listed market assets
    pub market_listing_fee: i128,
    /// Minimum value a trade must sell, in USD with 7 decimals (1 USD = `MIN_TRADE_SIZE_UNIT`),
    /// zero disables the limit
    pub min_trade_size: i128,
    /// Expected average ledger close time in seconds
    pub ledger_time: u32,
}
```

The listing fee pays one `track` call, whose amount the oracle splits evenly across the market's listed assets: at 90
days, a pair with one listed asset gets 90 days of that asset's feed, a pair with both assets listed 45 days of each.
`listing_min_days` therefore counts asset-days, and the fee is `daily_fee × listing_min_days` whatever the number of
listed assets. A later change of the oracle's own daily fee is picked up by the next `set_oracle` or
`set_listing_min_days`. With `listing_min_days` at zero the listing fee is zero and `subsidize` opens a market for any
positive amount, burning only what it is given; a zero or negative amount is `InvalidAmount` in every case.
Price feed access is bought per asset, so a market whose assets other markets already provision can value its orders
right away; one whose assets nobody provisions needs a subsidy before its orders can be valued.

Prices are cached per asset together with the decimals of the oracle that quoted them. After `set_oracle` the cached
prices keep valuing orders, until they reach the 72-hour age limit, when the new oracle quotes with the same decimals,
and read as missing when it does not. The new oracle has to be provisioned (`subsidize`) before `requote` can refresh
them.

## Frozen mode

The `safety_admin` can freeze the contract with `freeze(true)` and release it with `freeze(false)`. The contract holds
no funds between calls, so a full stop strands no user funds.

The contract in the `frozen` state blocks all trading and market operations (`trade`, `swap`, `crossfill`, `subsidize`,
`requote`) and any `update` that changes an order. Order removals and approvals through `update` still work, so makers
can pull their quotes and revoke allowances before trading resumes. A market whose assets the oracle no longer quotes
applies the same rule to its own orders: they stay fillable and removable, but `update` cannot change them
(`AssetsNotVerifiedByOracle`). Read-only functions and admin-level operations (`freeze`, `delegate`, `set_oracle`,
`set_floor`, `set_listing_min_days`, `set_ledger_time`) remain active.

Open orders and standing allowances are left untouched by the switch; an allowance can only be spent through the
contract, which cannot perform any balance actions while frozen.

## Contract lifetime

The contract has no lifetime entry point. Keepers extend the contract instance and its WASM code with the standard
`ExtendFootprintTTL` operation (both entries in the read-only footprint), for a horizon of their choice up to the network
maximum. Every state-changing entry point also extends both entries when less than 3 days are left, and then to 3 days,
so traders pay contract rent only when keepers have not done their job.

Every lifetime the contract sets (contract, markets, orders, cached prices) is given in days or hours and converted into
ledgers with `Config::ledger_time`, 5 seconds per ledger at deployment. When the network changes its ledger close time,
the safety admin updates it with `set_ledger_time` (1 to 20 seconds), so entries keep living for the intended time.

## Events

Trading events use the `vec` data format: the data is a vector of the fields in the order listed. Indexers derive
trade/swap id from the ledger, transaction and event position.

### `trade`

Emitted once per each order fill. Fills emit no order events.

Topics: `["trade", selling: Address, buying: Address]` (assets sold and bought by the taker)
Data: `[order: u128, taker: Address, maker: Address, sold: i128, bought: i128, left: i128]`
`left` is the order amount after the fill, `0` when the order was filled in full (or what is left became dust) and
removed.

### `swap`

Emitted once per `swap` call, after the fills of every hop.

Topics: `["swap", selling: Address, buying: Address]`
Data: `[trader: Address, sold: i128, bought: i128]`

### `new`

Emitted when a new order is created on the book.

Topics: `["new", selling: Address, buying: Address]`
Data: `[id: u128, owner: Address, price: i128, amount: i128, expires: u64]`

### `mod`

Emitted when an order changed outside a fill - by `update` with the new price, amount and expiration, and by a removal
(an `update` with a zero amount) with `amount = 0` and the price and expiration unchanged. An order reaching its
expiration emits nothing.

Topics: `["mod"]`
Data: `[id: u128, price: i128, amount: i128, expires: u64]`

### `skip`

Emitted when a listed order is not executed because its maker could not settle the fill: the backing left does not cover
it, the maker cannot receive the taker's asset (a missing, deauthorized or full trustline), or the maker's transfer
failed on the maker's side. A transfer the taker cannot receive fails the trade (`CannotReceive`) instead, so `skip`
never flags a maker for the taker's trustline. The order is left unchanged. For `crossfill` it also flags a taker order
its owner cannot back or be paid for. Missing, expired, overpriced and duplicate ids, and ids of another pair, emit
nothing.

Topics: `["skip"]`  
Data: `u128` - the order id

### `refresh`

Emitted every time a market's assets are checked against the oracle (`requote`, `subsidize`, market creation), whether
or not the market record changed.

Topics: `["refresh", base: Address, quote: Address]` (the market assets in canonical order)  
Data: none

### `freeze`

Emitted on every `freeze` call, when the contract is either frozen or unfrozen.

Topics: `["freeze"]`  
Data: `bool` - whether trading is blocked after the call

### `config`

Emitted when the configuration is set: by the constructor, `delegate`, `set_oracle`, `set_floor`,
`set_listing_min_days` and `set_ledger_time`.

Topics: `["config"]`  
Data: `Config` - the configuration after the call

## Limits

The per-transaction contract event cap (16,384 bytes) is the major limitation: a fill costs about 792 bytes of events
(one `trade` event and two token `transfer` events), plus one `transfer` per trade forwarding the makers' assets from
the contract to the taker, so a single `trade` can cross up to 20 maker orders from distinct makers. A skipped order
adds an 84-byte `skip` event: 20 fills leave room for 3 skipped orders, so routers budget listed orders, not only fills.
An `update` batch can hold about 110 orders (modified or removed). Write entries limit results in the max orders cap of
about 49 orders per trade.

Every `trade` and `swap` writes the contract's own balance of the asset it buys (and of every hop asset of a `swap`),
so trades buying the same asset share that ledger entry and cannot run in parallel. A `swap` forwards what its last
hop bought to the trader in one transfer as well, one `transfer` event per swap on top of the fills.

Assets whose issuer requires authorization (`AUTH_REQUIRED`) pass through the contract whenever they are bought:
`trade`, `crossfill` and every `swap` hop deliver the makers' assets to the contract before they reach the trader. The
issuer has to authorize the contract as well as the counterparties (`set_authorized(contract, true)` on the asset
contract); until then such a purchase fails with `IntermediaryCannotReceive`. Selling such an asset needs no contract
authorization, since the taker pays each maker directly.

The oracle may quote with at most 24 decimals, and an asset can be valued only when its token decimals and the oracle
decimals add up to 37 at most (23 token decimals under Reflector's 14); markets record an asset beyond that as
unlisted.

## Deployment and TS Bindings

Build a contract

```shell
stellar contract build --optimize
```

And create TS bindings

```shell
stellar contract bindings typescript --output-dir ./bindings --wasm target/wasm32v1-none/release/axis_markets_orderbook.wasm --overwrite --network-passphrase "Test SDF Network ; September 2015" --rpc-url https://soroban-testnet.stellar.org
```
