use actix_multipart::Multipart;
use actix_rt::blocking::BlockingError;
use actix_web::error::ErrorBadRequest;
use actix_web::{middleware, web, get, post, error, App, Error as AWError, HttpResponse, HttpServer};
use bytes::BytesMut;
use listenfd::ListenFd;
use mysql::MySqlError;
use mysql::prelude::{FromRow, Queryable};
use r2d2::PooledConnection;
use r2d2_mysql::MysqlConnectionManager;
// use futures::TrystreamExt;
// use listenfd::ListenFd;
// use mysql::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::stream::StreamExt;
use std::{clone, env};
// use std::cmp;
// use std::fs::File;
use std::sync::Arc;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

use crate::models::*;

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

const ItemsPerPage: usize = 48;
const TransactionsPerPage: i32 = 10;

const BcryptCost: i32 = 10;

const MAX_SIZE: usize = 262_144;
const DBConnectionCheckoutErrorMsg: &str = "Failed to checkout database connection";

mod models;

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
            .service(getNewItems)
            .service(getNewCategoryItems)
            // .service(getTransactions)
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

/*
 * Common functions
 */

fn getUserSimpleById(
    user_id: i64,
    db: &web::Data<Pool>,
) -> Result<Option<UserSimple>, mysql::Error> {
    let mut conn = db.get().expect("Failed to checkout database connection");
    conn.query_map(
        format!("SELECT id, account_name, num_sell_items FROM users WHERE id = {}", user_id),
        |(id, account_name, num_sell_items)|
        UserSimple {
            id, account_name, num_sell_items
        }
    )
    .map(|mut users| users.pop())
}

fn getImageUrl(image_name: &String) -> String {
    format!("/upload/{}", image_name)
}

fn getCategoryById(
    category_id: i32,
    db: &web::Data<Pool>,
) -> Result<Option<Category>, mysql::Error> {
    let mut conn = db.get().expect("Failed to checkout database connection");
    conn.query_map(
        format!("SELECT * FROM categories WHERE id = {}", category_id),
        |(id, parent_id, category_name)| {
            let parent_category_name = if parent_id == 0 {
                String::from("")
            } else {
                let parent_category = getCategoryById(parent_id, &db)
                .unwrap_or(None);
                match parent_category {
                    Some(p) => p.category_name,
                    None => String::from(""),
                }
            };
            Category {
                id, parent_id, category_name, parent_category_name
            }
        }
    )
    .map(|mut categories| categories.pop())
}

/*
 * API
 */

// region: index
#[get("/")]
async fn index() -> Result<HttpResponse, actix_web::Error> {
    let response_body = "Hello World!";
    Ok(HttpResponse::Ok().body(response_body))
}
// endregion

// region: initialize
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
        sql_dir.join("initial.sql"),
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
// endregion

// region: getNewItems
#[derive(Debug, Deserialize)]
struct GetNewItemsParams {
    pub item_id: Option<i64>,
    pub created_at: Option<i64>,
}

impl GetNewItemsParams {
    fn validate(&self) -> Result<(), HttpResponse> {
        match self.item_id {
            Some(item_id) => {
                if (0 < item_id) {
                    match self.created_at {
                        Some(created_at) => {
                            if (0 < created_at) {
                                Ok(())
                            } else {
                                Err(HttpResponse::BadRequest().json(
                                    BadRequestResponse {
                                        error: "created_at param error".to_string()
                                    }
                                ))
                            }
                        }
                        _ => Ok(())
                    }
                } else {
                    Err(HttpResponse::BadRequest().json(
                        BadRequestResponse {
                            error: "item_id param error".to_string()
                        }
                    ))
                }
            }
            _ => Ok(())
        }
    }
}

#[get("/new_items.json")]
async fn getNewItems(
    db: web::Data<Pool>,
    query_params: web::Query<GetNewItemsParams>,
) -> Result<HttpResponse, AWError> {
    let query_validation = query_params.validate();
    if (query_validation.is_err()) {
        return Ok(query_validation.unwrap_err())
    }


    let res = web::block(move || {
        let mut conn = db.get().expect("Failed to checkout database connection");

        let items = if query_params.item_id.is_some() && query_params.created_at.is_some() {
            let item_id = query_params.item_id.unwrap();
            let created_at = query_params.created_at.unwrap();
            // paging
            let query = format!(
                "SELECT * FROM items WHERE status IN ('{}', '{}') AND (created_at < from_unixtime({}) OR (created_at <= from_unixtime({}) AND id < {})) ORDER BY created_at DESC, id DESC LIMIT {}",
                ItemStatusOnSale,
                ItemStatusSoldOut,
                created_at,
                created_at,
                item_id,
                ItemsPerPage + 1,
            );
            conn.query_map(
                query,
                Item::from_row
            )
        } else {
            let query = format!(
                "SELECT * FROM items WHERE status IN ('{}', '{}') ORDER BY created_at DESC, id DESC LIMIT {}",
                ItemStatusOnSale,
                ItemStatusSoldOut,
                ItemsPerPage + 1,
            );
            conn.query_map(
                query,
                Item::from_row
            )
        }?;

        let mut item_simples: Vec<ItemSimple> = Vec::new();
        for item in items.iter() {
            let seller = getUserSimpleById(item.seller_id, &db)?;
            let category = getCategoryById(item.category_id, &db)?;
            if seller.is_some() && category.is_some() {
                item_simples.push(
                    ItemSimple {
                    id: item.id,
                    seller_id: item.seller_id,
                    seller: seller.unwrap(),
                    status: item.status.clone(),
                    name: item.name.clone(),
                    image_url: getImageUrl(&item.image_name),
                    category_id: item.category_id,
                    category: category.unwrap(),
                    created_at: item.created_at,
                });
            }
        }

        let has_next = item_simples.len() > ItemsPerPage;
        while item_simples.len() > ItemsPerPage  {
            item_simples.pop();
        }
        Ok(
            NewItemsResponse {
                root_category_id: None,
                root_category_name: None,
                items: item_simples,
                has_next: has_next,
            }
        )
    })
    .await
    .map_err(|e: BlockingDBError| {
        log::error!("getNewItems DB execution error : {:?}", e);
        HttpResponse::InternalServerError()
    })?;
    Ok(
        HttpResponse::Ok().json(res)
    )
}
// endregion

// region: getNewCategoryItems
#[derive(Debug, Deserialize)]
struct GetNewCategoryItemsParam {
    item_id: Option<i64>,
    created_at: Option<i64>,
}

impl GetNewCategoryItemsParam {
    fn validate(&self) -> Result<(), HttpResponse> {
        match self.item_id {
            Some(item_id) => {
                if (0 < item_id) {
                    match self.created_at {
                        Some(created_at) => {
                            if (0 < created_at) {
                                Ok(())
                            } else {
                                Err(HttpResponse::BadRequest().json(
                                    BadRequestResponse {
                                        error: "created_at param error".to_string()
                                    }
                                ))
                            }
                        }
                        _ => Ok(())
                    }
                } else {
                    Err(HttpResponse::BadRequest().json(
                        BadRequestResponse {
                            error: "item_id param error".to_string()
                        }
                    ))
                }
            }
            _ => Ok(())
        }
    }
}

#[get("/new_items/{root_category_id}.json")]
async fn getNewCategoryItems(
    db: web::Data<Pool>,
    path: web::Path<(i32)>,
    query_params: web::Query<GetNewCategoryItemsParam>,
)
-> Result<HttpResponse, AWError> {
    let query_validation = query_params.validate();
    if (query_validation.is_err()) {
        return Ok(query_validation.unwrap_err())
    }

    let (root_category_id) = path.into_inner();

    let res = web::block(move || {
        let mut conn = db.get().expect(DBConnectionCheckoutErrorMsg);
        let root_category = getCategoryById(root_category_id, &db)?;
        let category_ids: Vec<i32> = conn.query(
            format!("SELECT id FROM categories WHERE parent_id={}", root_category_id)
        )?;
        let category_ids_str: Vec<String> = category_ids
            .iter()
            .map(|id| format!("'{}'", id))
            .collect();
        let items: Vec<Item> = if category_ids.len() == 0 {
            Ok(Vec::new())
        } else if query_params.item_id.is_some() && query_params.created_at.is_some() {
            let item_id = query_params.item_id.unwrap();
            let created_at = query_params.created_at.unwrap();
            let sql = format!(
                "SELECT * FROM items
                WHERE status IN ('{}', '{}') AND
                category_id IN ({}) AND
                created_at < '{}'
                ORDER BY created_at DESC, id DESC LIMIT {}",
                ItemStatusOnSale,
                ItemStatusSoldOut,
                category_ids_str.join(","),
                NaiveDateTime::from_timestamp(created_at, 0),
                ItemsPerPage + 1
            );
            db.get().expect(DBConnectionCheckoutErrorMsg).query_map(
                sql,
                Item::from_row
            )
        } else {
            let sql = format!(
                "SELECT * FROM items
                WHERE status IN ('{}', '{}') AND category_id IN ({})
                ORDER BY created_at DESC, id DESC LIMIT {}",
                ItemStatusOnSale,
                ItemStatusSoldOut,
                category_ids_str.join(","),
                ItemsPerPage + 1
            );
            db.get().expect(DBConnectionCheckoutErrorMsg).query_map(
                sql,
                Item::from_row
            )
        }?;
        let mut item_simples: Vec<ItemSimple> = Vec::new();
        for item in items.iter() {
            let seller = getUserSimpleById(item.seller_id, &db)?;
            let category = getCategoryById(item.category_id, &db)?;
            if seller.is_some() && category.is_some() {
                item_simples.push(
                    ItemSimple {
                    id: item.id,
                    seller_id: item.seller_id,
                    seller: seller.unwrap(),
                    status: item.status.clone(),
                    name: item.name.clone(),
                    image_url: getImageUrl(&item.image_name),
                    category_id: item.category_id,
                    category: category.unwrap(),
                    created_at: item.created_at,
                });
            }
        }

        let has_next = item_simples.len() > ItemsPerPage;
        while item_simples.len() > ItemsPerPage  {
            item_simples.pop();
        }
        Ok(
            NewItemsResponse {
                root_category_id: Some(root_category_id),
                root_category_name: root_category.map(|op| op.category_name),
                items: item_simples,
                has_next: has_next,
            }
        )
    })
    .await
    .map_err(|e: BlockingDBError| {
        log::error!("getNewCategoryItems DB execution error {:?}", e);
        HttpResponse::InternalServerError()
    })?;

    Ok(HttpResponse::Ok().json(res))
}
// endregion

// region: getTransaction
// #[derive(Debug, Deserialize)]
// struct GetTransactionsRequest {
//     pub item_id: Option<i64>,
//     pub created_at: Option<i64>
// }

// impl GetTransactionsRequest {
//     fn isSome(&self) -> bool {
//         self.item_id.is_some() && self.created_at.is_some()
//     }

//     fn isOutOfRange(&self) -> bool {
//         match (self.item_id, self.created_at) {
//             (Some(item_id), Some(created_at)) => {
//                 0 < item_id && 0 < created_at
//             }
//             _ => true
//         }
//     }
// }

// #[get("/users/transactions.json")]
// async fn getTransactions(
//     db: web::Data<Pool>,
//     query_params: web::Query<GetTransactionsRequest>,
// ) -> Result<HttpResponse, AWError> {}
// endregion