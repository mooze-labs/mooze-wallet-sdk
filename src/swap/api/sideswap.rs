use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::infra::json_rpc;

mod error;
mod models;

use lwk_wollet::WalletTxOut;
pub use models::*;
pub use error::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;
use tokio::sync::{mpsc, RwLock, watch};
use tokio::time::{Duration, timeout};

const SIDESWAP_API_KEY: &str = "5c85504bf60e13e0d58614cb9ed86cb2c163cfa402fb3a9e63cf76c7a7af46a1";
const SIDESWAP_URL: &str = "wss://api.sideswap.io/json-rpc-ws";
const SIDESWAP_TESTNET_URL: &str = "wss://api-testnet.sideswap.io/json-rpc-ws";

const USER_AGENT: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const TIMEOUT_DURATION_SECS: u64 = 10;
const CHECK_INTERVAL_MILLIS: u64 = 100;

const ASSET_PRECISION: u64 = 10_u64.pow(8);

#[derive(Clone)]
struct ServerStatus {
    quote_tx: watch::Sender<Option<QuoteStatus>>,
    quote_rx: watch::Receiver<Option<QuoteStatus>>,
}

pub(crate) struct SideswapClient {
    is_connected: AtomicBool,
    rpc_client: Arc<json_rpc::JsonRpcClient>,
    server_status: ServerStatus,
}

impl SideswapClient {
    pub fn new(mainnet: bool) -> Result<Self, SideswapError> {
        let url = if mainnet { SIDESWAP_URL } else { SIDESWAP_TESTNET_URL };
        let (quote_tx, quote_rx) = watch::channel(Option::<QuoteStatus>::None);

        let server_status = ServerStatus {
            quote_tx,
            quote_rx
        };

        let rpc_client = Arc::new(json_rpc::JsonRpcClient::new(url)?);

        Ok(SideswapClient { 
            is_connected: AtomicBool::new(false),
            rpc_client, 
            server_status,
        })
    }

    async fn call_api<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
        result_key: &str,
    ) -> Result<T, SideswapError> {
        if !self.is_connected.load(Ordering::SeqCst) {
            return Err(SideswapError::ConnectionError("Must be connected.".to_string()));
        }

        let response = self.rpc_client
            .call_method(method, Some(params))
            .await?;

        let result = response.get("result");
        match result {
            None => {
                if let Some(err) = response.get("error") {
                    return Err(SideswapError::ApiResponseError(err.to_string()))
                } else {
                    return Err(SideswapError::MissingResultKey(result_key.to_string()))
                }
            },
            Some(r) => {
                if result_key.is_empty() {
                    let data: T = serde_json::from_value(r.clone())?;
                    return Ok(data);
                }
                let data: T = serde_json::from_value(r[result_key].clone())?;
                Ok(data)
            }
        }
    }

    pub async fn start(&mut self) -> Result<(), SideswapError> {
        self.wait_for_connection().await?;
        self.login().await?;
        self.start_notification_listener().await;

        *self.is_connected.get_mut() = true;

        Ok(())
    }

    async fn login(&self) -> Result<(), SideswapError> {
        let params = json!({
            "api_key": SIDESWAP_API_KEY,
            "user-agent": USER_AGENT,
            "version": VERSION,
        });

        self.rpc_client.call_method("login", Some(params)).await?;
        Ok(())
    }

    async fn wait_for_connection(&self) -> Result<(), SideswapError> {
        let timeout_duration = Duration::from_secs(TIMEOUT_DURATION_SECS);
        let check_interval = Duration::from_millis(CHECK_INTERVAL_MILLIS);

        timeout(timeout_duration, async {
            while !self.rpc_client.is_connected() {
                tokio::time::sleep(check_interval).await;
            }
        })
        .await
        .map_err(|e| SideswapError::ConnectionTimeout("Timed out waiting for connection".to_string()))?;
        
        Ok(())
    }

    async fn start_notification_listener(&self) {
        let rpc_client = self.rpc_client.clone();
        let server_status = self.server_status.clone();

        tokio::spawn(async move {
            loop {
                let notification = rpc_client.wait_for_notification().await;
                let _ = Self::process_notification(notification, &server_status).await;
            }
        });
    }

    async fn process_notification(notification: serde_json::Value, server_status: &ServerStatus) -> Result<(), SideswapError> {
        if let Some(method) = notification.get("method") {
            match method.as_str() {
                Some("market") => Self::process_market_notification(notification["params"].clone(), server_status).await,
                _ => { return Err(SideswapError::NotificationError(format!("Unknown notification type: {method}"))); }
            }
        }

        Err(SideswapError::ApiResponseError("Notification without method body.".to_string()))
    }

    async fn process_market_notification(params: serde_json::Value, server_status: &ServerStatus) {
        if let Some(quote) = params.get("quote") {
            let _ = Self::update_quote(server_status.quote_tx.clone(), quote).await;
        }
    }

    async fn update_quote(sender: watch::Sender<Option<QuoteStatus>>, quote: &serde_json::Value) -> Result<(), SideswapError> {
        let quote_sub_id = quote["quote_sub_id"]
            .as_i64()
            .ok_or_else(|| SideswapError::FormatError("Failed to parse quote ID.".to_string()))?;

        let status = match quote.get("status") {
            None => { return Err(SideswapError::FormatError("Status not available on quote.".to_string())); },
            Some(status) => Self::parse_quote_status(status)
        }?;

        let _ = sender.send(Some(status));

        Ok(())
    }

    fn parse_quote_status(status: &serde_json::Value) -> Result<QuoteStatus, SideswapError> {
        if let Some(low_balance) = status.get("LowBalance") {
            return Ok(Self::parse_low_balance_quote(low_balance));
        }

        if let Some(error) = status.get("Error") {
            return Ok(Self::parse_error_quote(error));
        }

        if let Some(success) = status.get("Success") {
            return Ok(Self::parse_success_quote(success));
        }

        Err(SideswapError::FormatError(format!("Quote has unknown type: {:?}", status)))
    }

    fn parse_success_quote(quote: &serde_json::Value) -> QuoteStatus {
        let quote = QuoteStatus::Success {
            quote_id: quote["quote_id"].as_u64().unwrap_or(0),
            base_amount: quote["base_amount"].as_u64().unwrap_or(0),
            quote_amount: quote["quote_amount"].as_u64().unwrap_or(0),
            server_fee: quote["server_fee"].as_u64().unwrap_or(0),
            fixed_fee: quote["fixed_fee"].as_u64().unwrap_or(0),
            ttl: quote["ttl"].as_u64().unwrap_or(0),
        };

        quote
    }

    fn parse_low_balance_quote(quote: &serde_json::Value) -> QuoteStatus {
        let quote = QuoteStatus::LowBalance {
            base_amount: quote["base_amount"].as_u64().unwrap_or(0),
            quote_amount: quote["quote_amount"].as_u64().unwrap_or(0),
            server_fee: quote["server_fee"].as_u64().unwrap_or(0),
            fixed_fee: quote["fixed_fee"].as_u64().unwrap_or(0),
            available: quote["available"].as_u64().unwrap_or(0),
        };

        quote
    }

    fn parse_error_quote(quote: &serde_json::Value) -> QuoteStatus {
        let quote = QuoteStatus::Error {
            error_msg: quote["error_msg"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_owned(),
        };

        quote
    }

    pub async fn peg(&self, peg_in: bool, address: &str) -> Result<PegOrder, SideswapError> {
        let peg_order: PegOrder = self.call_api("peg", json!({"peg_in": peg_in, "recv_addr": address}), "").await?;

        Ok(peg_order)
    }
    pub async fn request_swap(
        &self, 
        send_asset: &str, 
        recv_asset: &str, 
        amount: u64, 
        recv_address: &str,
        change_address: &str,
        utxos: Vec<WalletTxOut>
    ) -> Result<QuoteStatus, SideswapError> {
        let quote = self.fetch_quote(send_asset, recv_asset, amount, recv_address, change_address, utxos).await?;

        match quote {
            QuoteStatus::Error { error_msg } => Err(SideswapError::ApiResponseError(error_msg)),
            QuoteStatus::LowBalance { .. } => Err(SideswapError::LowBalance),
            QuoteStatus::Success { .. } => Ok(quote)
        }
    }

    /// Fetches the current rate for a given asset on the Sideswap market.
    /// Uses SideswapClient::fetch_quote() as underlying API call.
    pub async fn fetch_current_rate(&self, send_asset: &str, recv_asset: &str, recv_address: &str, change_address: &str) -> Result<f64, SideswapError> {
        let quote = match self.fetch_quote(send_asset, recv_asset, 1_u64 * ASSET_PRECISION, recv_address, change_address, Vec::new()).await? {
            QuoteStatus::Error { error_msg} => Err(SideswapError::ApiResponseError(error_msg)),
            QuoteStatus::LowBalance { base_amount, quote_amount, .. } => {
                let rate = (quote_amount as f64) / (base_amount as f64);
                return Ok(rate as f64);
            },
            QuoteStatus::Success { base_amount, quote_amount, .. } => {
                let rate = (quote_amount as f64) / (base_amount as f64);
                return Ok(rate as f64);
            }
        }?;

        quote
    }

    /// Requests a quote operation. Waits for a QuoteStatus to appear on the
    /// watch receiver and returns it.
    pub async fn fetch_quote(
        &self, 
        send_asset: &str, 
        recv_asset: &str, 
        amount: u64, 
        recv_address: &str,
        change_address: &str,
        utxos: Vec<WalletTxOut>
    ) -> Result<QuoteStatus, SideswapError> {
        let _ = self.start_quote(
            send_asset, 
            recv_asset,
            amount,
            recv_address, 
            change_address, 
            utxos
        ).await?;

        let mut rx = self.server_status.quote_rx.clone();

        let quote = timeout(
            Duration::from_secs(TIMEOUT_DURATION_SECS),
            rx.wait_for(|f| f.is_some())
        ).await
        .map_err(|e| SideswapError::ConnectionTimeout("Timeout waiting for quote.".to_string()))?
        .map_err(|e| SideswapError::ChannelSendError(e.to_string()))?
        .clone()
        .ok_or_else(|| SideswapError::ApiResponseError("Quote has not been received.".to_string()))?;

        Ok(quote)
    }

    async fn start_quote(
        &self, 
        send_asset: &str, 
        recv_asset: &str, 
        amount: u64, 
        recv_address: &str,
        change_address: &str,
        utxos: Vec<WalletTxOut>
    ) -> Result<(), SideswapError> {
        self.stop_quotes().await?;
        let market = self.get_market(send_asset, recv_asset).await?;
        let utxos = utxos.iter().map(|u| SideswapUtxo {
            txid: u.outpoint.txid.to_string(),
            vout: u.outpoint.vout,
            asset: u.unblinded.asset.to_string(),
            asset_bf: u.unblinded.asset_bf.to_string(),
            value: u.unblinded.value,
            value_bf: u.unblinded.value_bf.to_string(),
            redeem_script: None
        }).collect();

        let quote_request = QuoteRequest {
            asset_pair: market.asset_pair,
            asset_type: if market.asset_type == "Quote" { AssetType::Base } else { AssetType::Quote },
            trade_dir: TradeDir::Sell, 
            amount,
            utxos,
            receive_address: recv_address.to_string(),
            change_address: change_address.to_string(),
        };

        let quote: StartQuotes = self.call_api("market", json!({"start_quotes": quote_request}), "start_quotes").await?;
        Ok(())
    }

    pub async fn retrieve_quote_pset(&self, quote_id: u64) -> Result<String, SideswapError> {
        let quote: Quote = self.call_api("market", json!({"get_quote": {"quote_id": quote_id}}), "quote_id").await?;

        Ok(quote.pset)
    }

    pub async fn sign_quote(&self, quote_id: u64, pset: String) -> Result<String, SideswapError> {
        let taker_sign: TakerSign = self.call_api(
            "market", 
            json!({
            "taker_sign": {
                "quote_id": quote_id,
                "pset": pset
            }
            }),
            "taker_sign"
        ).await?;

        Ok(taker_sign.txid)
    }

    pub async fn stop_quotes(&self) -> Result<(), SideswapError> {
        self.rpc_client.call_method("stop_quotes", Some(json!({"params": {}}))).await?;
        
        Ok(())
    }

    async fn get_market(&self, send_asset: &str, recv_asset: &str) -> Result<Market, SideswapError> {
        let markets: ListMarkets = self.call_api(
            "market",
            json!({"list_markets": {}}),
            "list_markets"
        ).await?;

        let market = markets.markets.into_iter()
            .find(|market| {
                (market.asset_pair.base.to_string() == send_asset && market.asset_pair.quote.to_string() == recv_asset)
                ||
                (market.asset_pair.base.to_string() == recv_asset && market.asset_pair.quote.to_string() == send_asset)
            })
            .ok_or(SideswapError::MarketRetrievalError(format!("Market not found: send_asset = {send_asset}, recv_asset = {recv_asset}")))?;

        Ok(market)
    }

    async fn start_quotes(&self, quote_request: QuoteRequest) -> Result<StartQuotes, SideswapError> {
        let result: StartQuotes = self.call_api("market", json!({"start_quotes": quote_request}), "start_quotes").await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::{models::WalletConfig, swap::api::sideswap::SideswapClient};

    const LBTC_TESTNET_ID: &str = "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49";
    const USDT_TESTNET_ID: &str = "b612eb46313a2cd6ebabd8b7a8eed5696e29898b87a43bff41c94f51acef9d73";

    #[tokio::test]
    /// Tests connection, fetches a quote. In the background it validates that JSON-RPC is working as expected
    /// and that the swap API is being queried as expected.
    async fn test_sideswap_connection() {
        let mut sideswap = SideswapClient::new(false).expect("Failed to connect WebSocket client to Sideswap");
        sideswap.start().await.unwrap();

        let rate = sideswap.fetch_current_rate(
            LBTC_TESTNET_ID, 
            USDT_TESTNET_ID, 
            "tlq1qqdv7pntgzz5hhw7glwm28vfphmzalzzh37nl7j23lukht2u8y6vl37m7f86qzlwf29s4m63zrdnxycvf4fpmj4a3vcl2pfxv3", 
            "tlq1qq2kq6keslrpmcpup04wt0exw0hwpw0zt3pdqmn036s4fpf7agdp87t778v73v38az6733fup8zhrsuee3m9rwl52eehkm3u8g"
        ).await.expect("Failed to get rate.");

        println!("{:?}", rate);
    }

    #[tokio::test]
    async fn test_sideswap_peg() {
        let address = "lq1qqd99k5m0ywmq08hcwkynhepygp486jh89ez3m89rhmrt8fpds5hwd84xz6t2weaj84c7zse7aj47jy0u63ngy4zczurxztqw4";

        let mut sideswap = SideswapClient::new(true).unwrap();
        sideswap.start().await.unwrap();

        let peg = sideswap.peg(true, &address).await.unwrap();
        println!("Order ID: {:?}", &peg.order_id);
    }
}