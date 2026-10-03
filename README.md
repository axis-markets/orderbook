# `@axis-markets/orderbook`

> Stellar smart contract for AXIS limit orderbook DEX.

The contract holds no user funds. Makers and takers grant the contract a standing token allowance, and every fill is
settled with `transfer_from` between the two counterparties, the contract acting as the spender. An order is backed by
its owner's balance and allowance. A fill the maker cannot back, or whose payment the maker cannot receive, is skipped:
the order is left unchanged and a `skip` event tells indexers which maker failed to settle.

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

- Safety admin address, price oracle address, market listing fee, and minimum trade size

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
contract.

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
Hand the safety admin role over to another account.

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

`fn keepalive()`
Extend the contract instance and code lifetime to 180 days.

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
deauthorized trustline). A transfer the taker cannot receive is not the maker's fault: the trade fails with
`CannotReceive` and no maker is flagged. Matching never removes, trims or caps an order. Routers should compute the
effective depth per maker and asset as `min(balance, allowance)` shared across that maker's orders, treat the
allowance's `live_until_ledger` as an expiry, and use `skip` events to stop proposing orders of makers that fail to
settle.

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

## Order storage format

The `order` view returns the full record:

```rust
struct Order {
    //unique order identifier, derived from the owner and the client nonce
    pub id: u128,
    //selling token address
    pub selling: Address,
    //buying token address
    pub buying: Address,
    //amount left to sell
    pub amount: i128,
    //maker address
    pub owner: Address,
    //order price: buying per 1 selling, 18 decimals
    pub price: i128,
    //expiration timestamp (0 = no expiration)
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
    //asset to buy at this step
    pub asset: Address,
    //maker order ids to match
    pub orders: Vec<u128>,
}

struct Approval {
    //token the allowance is granted on
    pub asset: Address,
    //absolute allowance amount
    pub amount: i128,
    //ledger sequence the allowance lives until
    pub live_until: u32,
}

struct OrderUpdate {
    pub id: u128,
    //new amount to sell, 0 removes the order
    pub amount: i128,
    //new price (ignored for a removal)
    pub price: i128,
    //expiration timestamp, 0 = no expiration (ignored for a removal)
    pub expires: u64,
}
```

## Market storage format

```rust
struct MarketSide {
    //token contract address
    pub asset: Address,
    //whether the asset is quoted by the oracle
    pub listed: bool,
    //token decimals (fetched only for listed assets)
    pub decimals: u32,
}

struct Market {
    //market assets in canonical (sorted) order
    pub a: MarketSide,
    pub b: MarketSide,
    //creation timestamp
    pub created: u64,
}
```

The record changes only when a side's listing or decimals change. Every check against the oracle (`requote`,
`subsidize`, market creation) emits a `refresh` event, which marks the time of the last verification.

## Standard errors

```rust
enum OrderbookError {
    /// Only order owner can modify it
    NotAuthorized = 701,
    /// Insufficient balance on the trader account
    InsufficientBalance = 702,
    /// The allowance granted to the contract does not cover the order
    InsufficientAllowance = 703,
    /// The traded assets are invalid: `selling` equals `buying`, or a swap has no path or a non-positive amount
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
    /// No listed market asset has a usable cached price: none was fetched by `requote` or `subsidize`
    /// under the current oracle, or the cached one is older than 72 hours
    AssetPriceOracleFetchFailed = 722,
    /// The price oracle does not satisfy the contract requirements
    InvalidOracleConfig = 723,
    /// Contract is frozen
    Frozen = 730,
    /// Arithmetic invariant violated: a price calculation whose result does not fit i128, a negative operand or zero
    /// divisor, or a negative crossfill surplus. Other arithmetic overflows trap (the contract is built with
    /// `overflow-checks`) and surface as a host error rather than this code
    Overflow = 740,
}
```

## Configuration

```rust
struct Config {
    //account allowed to freeze the contract and change the minimum trade size
    pub safety_admin: Address,
    //Reflector Beam price oracle contract address
    pub oracle: Address,
    //XRF stroops burned by the market creator to provision the oracle price feeds: the oracle's daily per-asset fee
    //times 90, read from the oracle's `fee_config` by the constructor and by `set_oracle`, never supplied directly
    pub market_listing_fee: i128,
    //minimum value a trade must sell, in USD with 7 decimals (1 USD = 10000000), zero disables the limit
    pub min_trade_size: i128,
}
```

The listing fee pays one `track` call, whose amount the oracle splits evenly across the market's listed assets: a pair
with one listed asset gets 90 days of that asset's feed, a pair with both assets listed 45 days of each. A later change
of the oracle's own daily fee is picked up only by the next `set_oracle`.

## Frozen mode

The `safety_admin` can freeze the contract with `freeze(true)` and release it with `freeze(false)`. The contract holds
no funds, so a full stop strands no user funds.

The contract in the `frozen` state blocks all trading and market operations (`trade`, `swap`, `crossfill`, `subsidize`,
`requote`) and any `update` that changes an order. Order removals and approvals through `update` still work, so makers
can pull their quotes and revoke allowances before trading resumes. Read-only functions, `keepalive` and admin-level
operations (`freeze`, `delegate`, `set_oracle`, `set_floor`) remain active.

Open orders and standing allowances are left untouched by the switch; an allowance can only be spent through the
contract, which cannot perform any balance actions while frozen.

## Contract lifetime

`keepalive` extends the contract instance and code to 180 days on every call; keepers are expected to call it regularly.
Every other entry point extends them only when less than 3 days are left, and then to 3 days, so traders pay contract
rent only when keepers have not done their job.

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
it, the maker cannot receive the taker's asset, or the maker's transfer failed on the maker's side. A transfer the taker
cannot receive fails the trade (`CannotReceive`) instead, so `skip` never flags a maker for the taker's trustline. The
order is left unchanged. For `crossfill` it also flags a taker order its owner cannot back or be paid for. Missing,
expired, overpriced and duplicate ids, and ids of another pair, emit nothing.

Topics: `["skip"]`  
Data: `u128` - the order id

### `refresh`

Emitted every time a market's assets are checked against the oracle (`requote`, `subsidize`, market creation), whether
or not the market record changed.

Topics: `["refresh", a: Address, b: Address]` (the market assets in canonical order)  
Data: none

### `freeze`

Emitted on every `freeze` call, when the contract is either frozen or unfrozen.

Topics: `["freeze"]`  
Data: `bool` - whether trading is blocked after the call

### `config`

Emitted when the configuration is set: by the constructor, `delegate`, `set_oracle` and `set_floor`.

Topics: `["config"]`  
Data: `Config` - the configuration after the call

## Limits

The per-transaction contract event cap (16,384 bytes) is the major limitation: a fill costs about 792 bytes of events
(one `trade` event and two token `transfer` events), so a single `trade` can cross up to 20 maker orders from distinct
makers. A skipped order adds an 84-byte `skip` event: 20 fills leave room for 6 skipped orders, so routers budget listed
orders, not only fills. An `update` batch can hold about 110 orders (modified or removed). Write entries limit results
in the max orders cap of about 49 orders per trade.

## Deployment and TS Bindings

Build a contract

```shell
stellar contract build --optimize
```

And create TS bindings

```shell
stellar contract bindings typescript --output-dir ./bindings --wasm target/wasm32v1-none/release/axis_markets_orderbook.wasm --overwrite --network-passphrase "Test SDF Network ; September 2015" --rpc-url https://soroban-testnet.stellar.org
```
