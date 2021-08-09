use actix_multipart::Multipart;
use actix_web::{middleware, web, get, post, error, App, Error as AWError, HttpResponse, HttpServer};
use listenfd::ListenFd;
use mysql::prelude::Queryable;
// use bytes::BytesMut;
// use futures::TrystreamExt;
// use listenfd::ListenFd;
// use mysql::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::stream::StreamExt;
use std::{clone, env};
// use std::cmp;
// use std::fs::File;
use std::sync::Arc;

type Pool = r2d2::Pool<r2d2_mysql::MysqlConnectionManager>;
type BlockingDBError = actix_web::error::BlockingError<mysql::Error>;

const sessionName: &str = "session_isucari";

const DefaultPaymentServiceURL: &str  = "http://localhost:5555";
const DefaultShipmentServiceURL: &str = "http://localhost:7000";

const ItemMinPrice: i32 = 100;
const ItemMaxPrice: i32 = 1000000;
const ItemPriceErrMsg: &str = "商品価格は100ｲｽｺｲﾝ以上、1,000,000ｲｽｺｲﾝ以下にしてください";

const ItemStatusOnSale: &str  = "on_sale";
const ItemStatustrading: &str = "trading";
const ItemStatusSoldOut: &str = "sold_out";
const ItemStatusStop: &str    = "stop";
const ItemStatusCancel: &str  = "cancel";

const PaymentServiceIsucariAPIKey: &str = "a15400e46c83635eb181-946abb51ff26a868317c";
const PaymentServiceIsucariShopID: &str = "11";

const TransactionEvidenceStatusWaitShipping: &str = "wait_shipping";
const TransactionEvidenceStatusWaitDone: &str = "wait_done";
const TransactionEvidenceStatusDone: &str = "done";

const ShippingsStatusInitial: &str = "initial";
const ShippingsStatusWaitPickup: &str = "wait_pickup";
const ShippingsStatusShipping: &str = "shipping";
const ShippingsStatusDone: &str = "done";

const BumpChargeSeconds: i32 = 3;

const ItemsPerPage: i32 = 48;
const TransactionsPerPage: i32 = 10;

const BcryptCost: i32 = 10;

const MAX_SIZE: usize = 262_144;

#[derive(Debug)]
struct MySQLConnectionEnv {
    host: String,
    port: u16,
    user: String,
    db_name: String,
    password: String,
}

impl Default for MySQLConnectionEnv {
    fn default() -> Self {
        let port = if let Ok(port) = env::var("MYSQL_PORT") {
            port.parse().unwrap_or(3306)
        } else {
            3306
        };
        Self {
            host: env::var("MYSQL_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned()),
            port,
            user: env::var("MYSQL_USER").unwrap_or_else(|_| "isucon".to_owned()),
            db_name: env::var("MYSQL_DBNAME").unwrap_or_else(|_| "isucari".to_owned()),
            password: env::var("MYSQL_PASS").unwrap_or_else(|_| "isucon".to_owned())
        }
    }
}

#[actix_rt::main]
async fn main() -> Result<(), std::io::Error> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "actix_server=info,actix_web=info,iscari=info");
    }
    env_logger::init();

    let mysql_connection_env = Arc::new(MySQLConnectionEnv::default());
    let manager = r2d2_mysql::MysqlConnectionManager::new(
        mysql::OptsBuilder::new()
        .ip_or_hostname(Some(&mysql_connection_env.host))
        .tcp_port(mysql_connection_env.port)
        .user(Some(&mysql_connection_env.user))
        .db_name(Some(&mysql_connection_env.db_name))
        .pass(Some(&mysql_connection_env.password)),
    );
    let pool = r2d2::Pool::builder()
        .max_size(10)
        .build(manager)
        .expect("Failed to create connection pool");

    let server = HttpServer::new(move ||
        App::new()
            .data(pool.clone())
            .data(mysql_connection_env.clone())
            .wrap(middleware::Logger::default())
            .service(index)
            .service(initialize)
        );
    let mut listenfd = ListenFd::from_env();
    let server = if let Some(l) = listenfd.take_tcp_listener(0)? {
        server.listen(l)?
    } else {
        server.bind((
            "0.0.0.0",
            std::env::var("SERVER_PORT")
                .map(|port_str| port_str.parse().expect("Failed to parse SERVER_PORT"))
                .unwrap_or(1323),
        ))?
    };
    server.run().await
}

#[get("/")]
async fn index() -> Result<HttpResponse, actix_web::Error> {
    let response_body = "Hello World!";
    Ok(HttpResponse::Ok().body(response_body))
}

#[derive(Deserialize)]
struct InitializeRequest {
    payment_service_url: String,
    shipment_service_url: String,
}

impl Default for InitializeRequest {
    fn default() -> Self {
        Self {
            payment_service_url: DefaultPaymentServiceURL.to_string(),
            shipment_service_url: DefaultShipmentServiceURL.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct InitializeResponse {
    campaign: i32,
    language: String,
}

#[post("/initialize")]
async fn initialize(
    mysql_connection_env: web::Data<Arc<MySQLConnectionEnv>>,
    db: web::Data<Pool>,
    mut payload: web::Payload,
) -> Result<HttpResponse, AWError> {
    // Initialize DB
    let sql_dir = std::path::Path::new("..").join("sql");
    let paths = [
        sql_dir.join("01_schema.sql"),
        sql_dir.join("02_categories.sql"),
    ];
    for p in paths.iter() {
        let sql_file = p.canonicalize().unwrap();
        let cmd_str = format!(
            "mysql -h {} -P {} -u {} -p{} {} < {}",
            mysql_connection_env.host,
            &mysql_connection_env.port,
            mysql_connection_env.user,
            &mysql_connection_env.password,
            mysql_connection_env.db_name,
            sql_file.display()
        );
        log::info!("run cmd {}", cmd_str);
        let status = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(cmd_str)
            .status()
            .await
            .map_err(|e| {
                log::error!("Initialize script {} failed : {:?}", p.display(), e);
                HttpResponse::InternalServerError()
            })?;
        if !status.success() {
            log::error!("Initialize script {} failed", p.display());
            return Ok(HttpResponse::InternalServerError().finish());
        }
    }

    // Update external service url
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk?;
        if (body.len() + chunk.len()) > MAX_SIZE {
            return Err(error::ErrorBadRequest("overflow"));
        }
        body.extend_from_slice(&chunk);
    }
    let req = serde_json::from_slice::<InitializeRequest>(&body).unwrap_or_default();
    web::block(move || {
        let mut conn = db.get().expect("Failed to checkout database connection");
        let mut tx = conn.start_transaction(mysql::TxOpts::default())?;
        let sql = "INSERT INTO configs (name, val) VALUES (?, ?) ON DUPLICATE KEY UPDATE val = VALUES(val)";
        let payment_service_url = req.payment_service_url.clone();
        let shipment_service_url = req.shipment_service_url.clone();
        tx.exec_drop(sql, ("payment_service_url", payment_service_url));
        tx.exec_drop(sql, ("shipment_service_url", shipment_service_url));
        tx.commit()?;
        Ok(())
    }).await.map_err(
        |e: BlockingDBError| {
            log::error!("Failed to insert/commit external service url: {:?}", e);
            HttpResponse::InternalServerError()
        },
    )?;

    Ok(HttpResponse::Ok().json(InitializeResponse {
        campaign: 0,
        language: "rust".to_owned(),
    }))
}
