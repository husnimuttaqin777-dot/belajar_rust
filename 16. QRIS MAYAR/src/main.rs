//! Minimal Mayar.id API client.
//! Docs: https://docs.mayar.id/api-reference

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::env;

const DEFAULT_BASE_URL: &str = "https://api.mayar.id/hl/v1";
const KEY: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiJiMTg5OWM0My01YjJkLTRlOGUtYmMyZS1jZDI2NGI3ZTE0ZDgiLCJhY2NvdW50SWQiOiI5NTIzNWFlYy1iNjJjLTQ0NjMtYjJlNC1kYzUwZWZkZWYyYWMiLCJjcmVhdGVkQXQiOiIxNzc2MzI3MDM2NTg5Iiwicm9sZSI6ImRldmVsb3BlciIsInN1YiI6Imh1c25pbXV0dGFxaW43NzdAZ21haWwuY29tIiwibmFtZSI6ImtlbGFzcm9ib3QiLCJsaW5rIjoiaHVzbmktbXV0dGFxaW4iLCJpc1NlbGZEb21haW4iOm51bGwsImlhdCI6MTc3NjMyNzAzNn0.Jbuaxfd0eDlgGwPHvDQc0HtIoPYqh1lbnj85MTVqzBb80mTZW3uWDRxI69NvTIG62fJ0egLKmTER9TXkz8M1Q4NajFvAHzE2NElk2qJSsM-QEcBhKvzrF7Zwupap3SfrSamqkt4qVv-dP19oo66cLKag32QmPNUab2_ZtdcQLefoldva6oTc1EQZRPvLA953zS3i6gtj42fJpDjWtm7-PIp0dXJTkWj51_Fdv_ENkr04oSC2cygnYHPOBUheYfaOYIpVlT45O6yH1jPxQt8gTR-RThsTUuywQu60fbggEhxc9f3H1MNx3Gch32K3-cnJBvGyuVmFr5kJPU207sEcmQ";

pub struct Mayar {
    base_url: String,
    client: Client,
}

impl Mayar {
    /// api_key: API key Mayar (Bearer token)
    /// base_url: production https://api.mayar.id/hl/v1
    ///           sandbox https://api.mayar.club/hl/v1
    pub fn new(api_key: &str, base_url: Option<&str>) -> Result<Self, reqwest::Error> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", api_key))
                .expect("API key mengandung karakter tidak valid"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder().default_headers(headers).build()?;

        Ok(Self {
            base_url: base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/').to_string(),
            client,
        })
    }

    /// GET /balance -> {statusCode, messages, data:{balanceActive, balancePending, balance}}
    pub fn balance(&self) -> Result<Value, reqwest::Error> {
        self.client
            .get(format!("{}/balance", self.base_url))
            .send()?
            .json()
    }

    /// POST /qrcode/create -> {statusCode, messages, data:{url, amount}}
    pub fn create_qris(&self, amount: i64) -> Result<Value, reqwest::Error> {
        self.client
            .post(format!("{}/qrcode/create", self.base_url))
            .json(&json!({ "amount": amount }))
            .send()?
            .json()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ambil key dari environment variable, fallback ke KEY
    let api_key = env::var("MAYAR_API_KEY").unwrap_or_else(|_| KEY.to_string());
    if api_key.is_empty() {
        eprintln!("Set MAYAR_API_KEY environment variable first.");
        std::process::exit(1);
    }

    let api = Mayar::new(&api_key, None)?;

    // Buat QRIS untuk Rp 10
    
    let qris = api.create_qris(10)?;
    if qris["statusCode"] == 200 {
        println!("QRIS URL: {}", qris["data"]["url"].as_str().unwrap_or(""));
    } else {
        println!("Error: {}", qris["messages"]);
    }
    

    // Cek saldo
    let balance = api.balance()?;
    if balance["statusCode"] == 200 {
        println!("Balance: {}", balance["data"]["balance"]);
    } else {
        println!("Error: {}", balance["messages"]);
    }

    Ok(())
}