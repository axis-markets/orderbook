# `axis-orderbook`

> Stellar smart contract for AXIS limit orderbook DEX.

## Interface

`fn __constructor(e: Env)`
Initialize the contract (called automatically on deployment)

---

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

`fn sell_limit(trader: Address, amount: i128, selling: Address, buying: Address, price: i128, orders: Vec<u64>) -> (i128, i128, u64)`
Trade with DEX and create sell limit order if quote not executed in full

Arguments
- `trader` - Trader address
- `amount` - Amount of tokens to sell
- `selling` - Selling token address
- `buying` - Buying token address
- `price` - Min price a trader willing to accept
- `orders` - Optional list of order IDs to match before creating the order on chain

Returns
- Amount of sold tokens
- Amount of bought tokens
- ID of the newly created order if any

Panics
- If the trader doesn't have sufficient balance
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

`fn fill(trader: Address, amount: i128, selling: Address, buying: Address, max_price: i128, orders: Vec<u64>) -> (i128, i128)`
Trade with orders

Arguments
- `trader` - Trader address
- `amount` - Amount of tokens to sell
- `selling` - Selling token address
- `buying` - Buying token address
- `max_price` - Max price a trader willing to pay
- `orders` - List of order IDs to match before creating the order on chain

Returns
- Amount of sold tokens
- Amount of bought tokens

Panics
- If any of the orders provided do not match selling/buying asset
- If the trade causes an overflow

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
enum OrderType {
    Limit
}

struct Order {
    pub id: u64,
    //order type
    pub kind: OrderType,
    //selling token address
    pub selling: Address,
    //buying token address
    pub buying: Address,
    //selling amount left
    pub amount: i128,
    //initial selling amount
    pub quote: i128,
    //maker address
    pub owner: Address,
    //order price
    pub price: i128,
    //expiration timestamp
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
