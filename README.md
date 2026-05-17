# `@axis-markets/orderbook`

> Stellar smart contract for AXIS limit orderbook DEX.

## Interface

`fn last() -> u64`
Get last order id

Returns
- Last created order id

---

`fn order(id: u64) -> Option<Order>`
Fetch existing order

Arguments
- `id` - ID of the order to fetch

Returns
- Order fetched from the storage

---

`fn sell(kind: OrderKind, trader: Address, amount: i128, selling: Address, buying: Address, price: i128, orders: Vec<u64>) -> (i128, i128, u64)`
Trade with DEX using one of the supported order types

Arguments
- `kind` - Order type: `Limit` (create order if not filled), `Fill` (fill only, no order created), `FillOrKill` (cancel if not fully filled)
- `trader` - Trader address
- `amount` - Amount of tokens to sell
- `selling` - Selling token address
- `buying` - Buying token address
- `price` - Price the trader willing to accept
- `orders` - Optional list of order IDs to match before creating the order on-chain

Returns
- Amount of sold tokens
- Amount of bought tokens
- ID of the newly created order (0 if no order was created)

Panics
- If the trader has insufficient balance
- If any of the orders provided do not match selling/buying asset
- If the trade causes an overflow

---

`fn buy(kind: OrderKind, trader: Address, amount: i128, buying: Address, selling: Address, price: i128, orders: Vec<u64>) -> (i128, i128, u64)`
Trade with DEX using one of the supported order types — buy-side variant. Matches against existing sell orders, terminating when the target buy amount is reached. Any unused worst-case deposit stays in the trader's wallet (immediate fills route selling tokens directly trader → maker, debited only for the amount actually consumed).

Arguments
- `kind` - Order type: `Limit` (create order if not filled), `Fill` (fill only, no order created), `FillOrKill` (cancel if not fully filled)
- `trader` - Trader address
- `amount` - Amount of buying tokens to acquire
- `buying` - Token to acquire
- `selling` - Token used to pay
- `price` - Maximum price (selling tokens per 1 buying token, PRECISION fixed-point) the trader is willing to pay
- `orders` - Optional list of order IDs to match before creating the order on-chain

Returns
- Amount of sold tokens (selling-side, actual spent)
- Amount of bought tokens (buying-side, acquired)
- ID of the newly created order (0 if no order was created)

Panics
- If the trader has insufficient balance to cover the worst-case deposit
- If any of the orders provided do not match selling/buying asset
- If the trade causes an overflow

---

`fn cancel(id: u64, trader: Address)`
Cancel existing order

Arguments
- `id` - ID of the order to cancel
- `trader` - Trader address

Panics
- If trader is not the owner of the order

---

`fn fill_order(trader: Address, taker_order_id: u64, orders: Vec<u64>) -> (i128, i128)`
Fill existing orders using another matching order from the orderbook

Arguments
- `trader` - Trader address
- `taker_order_id` - ID of the order that serves as a taker
- `orders` - List of order IDs to match before creating the order on chain

Returns
- Amount of sold tokens
- Amount of bought tokens

Panics
- If the taker order was not found
- If any of the orders provided do not match selling/buying asset
- If the trade causes an overflow

## Order storage format

```rust
enum OrderKind {
    Limit = 1,
    Fill = 2,
    FillOrKill = 3,
}

struct Order {
    pub id: u64,
    //order type
    pub kind: OrderKind,
    //selling token address
    pub selling: Address,
    //buying token address
    pub buying: Address,
    //amount left to sell/buy
    pub amount: i128,
    //initial selling/buying amount
    pub quote: i128,
    //maker address
    pub owner: Address,
    //order price
    pub price: i128,
    //expiration timestamp (0 = no expiration)
    pub expires: u64
}
```

## Trade format

```rust
struct Trade {
    pub id: u64,
    //order id
    pub order: u64,
    //trader account address
    pub taker: Address,
    //seller account address
    pub maker: Address,
    //sold asset address
    pub selling: Address,
    //bought asset address
    pub buying: Address,
    //sold tokens amount
    pub sold: i128,
    //bought tokens amount
    pub bought: i128,
}
```

## Standard errors

```rust
enum OrderbookError {
    NotAuthorized = 701,
    InsufficientBalance = 702,
    OrderNotFound = 703,
    Overflow = 704,
    InvalidMatch = 705,
    InvalidPrice = 706,
}
```

## Events

**OrderEvent**

Topics: `["AXIS", "order", action: Symbol, selling: Address, buying: Address]`  
Body: `Order`  
Action: one of `"created"`|`"updated"`|`"removed"`


**TradeEvent**

Topics: `["AXIS", "trade", selling: Address, buying: Address]`  
Body: `Trade`


## Deployment and TS Bindings

Build a contract

```shell
stellar contract build --optimize
```

Deploy it to the network, obtain contract ID.

And create TS bindings

```shell
stellar contract bindings typescript --output-dir ./bindings --contract-id {contract_id} --overwrite --network-passphrase "Test SDF Network ; September 2015" --rpc-url https://soroban-testnet.stellar.org
```