use crate::orderbook::invert_price;
use crate::{SorobanOrderbook, SorobanOrderbookClient, PRECISION};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};
use test_case::test_case;

//rounding
#[test_case(3, 2, 3000, 4501, 3000, 4501)]
#[test_case(3, 2, 3000, 4500, 2999, 4500)]
#[test_case(3, 2, 3000, 4499, 2999, 4499)]
#[test_case(3, 2, 2999, 4499, 2999, 4499)]
#[test_case(3, 2, 2999, 4498, 2998, 4498)]
#[test_case(2, 3, 3000, 2001, 3000, 1999)]
#[test_case(2, 3, 3000, 2000, 3000, 2000)]
#[test_case(2, 3, 3000, 1999, 2998, 1999)]
#[test_case(2, 3, 2999, 2000, 2999, 1999)]
#[test_case(2, 3, 2999, 1999, 2998, 1999)]
//micro trades
#[test_case(3, 2, 28, 27, 17, 27)]
#[test_case(3, 2, 28, 26, 17, 26)]
#[test_case(3, 2, 52, 51, 33, 51)]
#[test_case(3, 2, 52, 50, 33, 50)]
#[test_case(30000000, 2, 1, 1, 0, 0)]
#[test_case(3, 20000000, 1, 1, 0, 0)]
#[test_case(30000000, 2, 10, 100000000, 6, 100000000)]
#[test_case(3, 20000000, 10000000, 10, 10000000, 1)]
#[test_case(3, 20000000, 10000000, 1, 6666666, 1)]
fn test_fill(
    n: i128,
    d: i128,
    x_order_amount: i128,
    y_trade_amount: i128,
    expected_x_received: i128,
    expected_y_sent: i128,
) {
    let e = Env::default();
    e.mock_all_auths();

    let maker = Address::generate(&e);
    let trader = Address::generate(&e);
    let issuer = Address::generate(&e);
    let usd = fake_asset(&e, &issuer);
    let eur = fake_asset(&e, &issuer);

    let usd_asset_client = StellarAssetClient::new(&e, &usd);
    let eur_asset_client = StellarAssetClient::new(&e, &eur);

    usd_asset_client.mint(&maker, &10000000000000000);
    eur_asset_client.mint(&maker, &10000000000000000);

    usd_asset_client.mint(&trader, &10000000000000000);
    eur_asset_client.mint(&trader, &10000000000000000);

    let price = PRECISION * d / n;

    let contract_address = e.register(SorobanOrderbook, ());
    let orderbook_client = SorobanOrderbookClient::new(&e, &contract_address);

    let (_, _, order_id) = orderbook_client.sell_limit(
        &maker,
        &x_order_amount,
        &usd,
        &eur,
        &price,
        &100,
        &Vec::new(&e),
    );
    let orders = Vec::from_array(&e, [order_id]);
    let (bought, sold) = orderbook_client.fill(
        &trader,
        &y_trade_amount,
        &eur,
        &usd,
        &invert_price(&e, price),
        &orders,
    );
    assert_eq!(
        bought, expected_y_sent,
        "trade {}/{} ({}X on order) {}Y -> ({}Y -> {}X)",
        n, d, x_order_amount, y_trade_amount, expected_y_sent, expected_x_received
    );
    assert_eq!(
        sold, expected_x_received,
        "trade {}/{} ({}X on order) {}Y -> ({}Y -> {}X)",
        n, d, x_order_amount, y_trade_amount, expected_y_sent, expected_x_received
    );
}

fn fake_asset(env: &Env, issuer: &Address) -> Address {
    env.register_stellar_asset_contract_v2(issuer.clone())
        .address()
}
