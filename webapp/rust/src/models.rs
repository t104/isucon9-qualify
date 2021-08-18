use mysql::{FromRowError, from_row_opt, prelude::FromRow};
use serde::{Deserialize, Serialize};
use bytes::BytesMut;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

pub struct User {
    pub id: i64,
    pub account_name: String,
    pub hashed_password: BytesMut,
    pub address: String,
    pub num_sell_items: i32,
    pub last_bump: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct UserSimple {
    pub id: i64,
    pub account_name: String,
    pub num_sell_items: i32,
}

#[derive(Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub seller_id: i64,
    pub buyer_id: i64,
    pub status: String,
    pub name: String,
    pub price: i32,
    pub description: String,
    pub image_name: String,
    pub category_id: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FromRow for Item {
    fn from_row(row: mysql::Row) -> Item {
        match from_row_opt(row) {
            Ok(i) => i,
            Err(err) => panic!("Convert row error: {:?} to Item", err),
        }
    }

    fn from_row_opt(row: mysql::Row) -> Result<Item, mysql::FromRowError> {
        mysql::from_row_opt(row).map(|(
            id,
            seller_id,
            buyer_id,
            status,
            name,
            price,
            description,
            image_name,
            category_id,
            created_at,
            updated_at)| {
            Item {
                id: id,
                seller_id: seller_id,
                buyer_id: buyer_id,
                status: status,
                name: name,
                price: price,
                description: description,
                image_name: image_name,
                category_id: category_id,
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            }
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct ItemSimple {
    pub id: i64,
    pub seller_id: i64,
    pub seller: UserSimple,
    pub status: String,
    pub name: String,
    pub image_url: String,
    pub category_id: i32,
    pub category: Category,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct ItemDetail {
    pub id: i64,
    pub seller_id: i64,
    pub seller: UserSimple,
    pub buyer_id: i64,
    pub buyer: UserSimple,
    pub status: String,
    pub name: String,
    pub price: String,
    pub description: String,
    pub image_url: String,
    pub category_id: i32,
    pub category: Category,
    pub transaction_evidence_id: i64,
    pub transacton_evidence_status: String,
    pub shipping_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct TransactionEvidence {
    pub id: i64,
    pub seller_id: i64,
    pub buyer_id: i64,
    pub status: String,
    pub item_id: i64,
    pub item_name: String,
    pub item_price: i32,
    pub item_description: String,
    pub item_category_id: i32,
    pub item_root_category_id: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct Shipping {
    pub transaction_evidence_id: i64,
    pub status: String,
    pub item_name: String,
    pub item_id: i64,
    pub reserve_id: i64,
    pub to_address: String,
    pub to_name: String,
    pub from_address: String,
    pub from_name: String,
    pub img_binary: BytesMut,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct Category {
    pub id: i32,
    pub parent_id: i32,
    pub category_name: String,
    pub parent_category_name: String,
}

#[derive(Serialize, Deserialize)]
pub struct NewItemsResponse {
    pub root_category_id: Option<i32>,
    pub root_category_name: Option<String>,
    pub has_next: bool,
    pub items: Vec<ItemSimple>,
}

#[derive(Serialize, Deserialize)]
pub struct TransactionsResponse {
    pub has_next: bool,
    pub items: Vec<ItemDetail>
}

#[derive(Serialize, Deserialize)]
pub struct RegisterRequest {
    account_name: String,
    address: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
pub struct LoginRequest {
    pub account_name: String,
    pub password: String,
}

#[derive(Serialize, Deserialize)]
pub struct ItemEditRequest {
    pub csrf_token: String,
    pub item_id: i64,
    pub item_price: i32,
}

#[derive(Serialize, Deserialize)]
pub struct BuyRequest {
    pub csrf_token: String,
    pub item_id: i64,
    pub token: String,
}

#[derive(Serialize, Deserialize)]
pub struct SellRequest {
    pub id: i64,
}

#[derive(Serialize, Deserialize)]
pub struct PostShipRequest {
    pub csrf_token: String,
    pub item_id: i64,
}

#[derive(Serialize, Deserialize)]
pub struct PostShipResponse {
    pubpath: String,
    pubreserve_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct PostShipDoneRequest {
    pub csrf_token: String,
    pub item_id: i64,
}

#[derive(Serialize, Deserialize)]
pub struct PostCompleteRequest {
    pub csrf_token: String,
    pub item_id: i64,
}

#[derive(Serialize, Deserialize)]
pub struct BumpRequest {
    pub csrf_token: String,
    pub item_id: i64,
}

pub struct SettingResponse {
    pub csrf_token: String,
    pub payment_service_url: String,
    pub user: User,
    pub categories: Vec<Category>,
}

#[derive(Debug, Serialize)]
pub struct BadRequestResponse {
    pub error: String
}