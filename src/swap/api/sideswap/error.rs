use thiserror::Error;

use crate::infra::json_rpc;

#[derive(Error, Debug)]
pub enum SideswapError {
    #[error("WebSocket client error: {0}")]
    WebSocketError(#[from] json_rpc::RpcError),
    #[error("Deserialization error: {0}")]
    DeserializationError(#[from] serde_json::Error),
    #[error("Low balance.")]
    LowBalance,
    #[error("Connection error: {0}.")]
    ConnectionError(String),
    #[error("Format error: {0}")]
    FormatError(String),
    #[error("Notification error: {0}")]
    NotificationError(String),
    #[error("Login failed: {0}")]
    LoginError(String),
    #[error("Missing result key: {0}")]
    MissingResultKey(String),
    #[error("Sideswap API error response: {0}")]
    ApiResponseError(String),
    #[error("Failed to get markets: {0}")]
    MarketRetrievalError(String),
    #[error("Failed to start quotes: {0}")]
    QuoteStartError(String),
    #[error("Failed to get quote: {0}")]
    QuoteRetrievalError(String),
    #[error("Failed to sign quote: {0}")]
    QuoteSigningError(String),
    #[error("Missing quote_sub_id in notification")]
    MissingQuoteSubId,
    #[error("Channel send error: {0}")]
    ChannelSendError(String),
    #[error("Connection timeout: {0}")]
    ConnectionTimeout(String),
}