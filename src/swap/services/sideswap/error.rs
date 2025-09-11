use thiserror::Error;

#[derive(Error, Debug)]
pub enum SideswapError {
    #[error("WebSocket client error: {0}")]
    WebSocketError(String),
    #[error("Login failed: {0}")]
    LoginError(String),
    #[error("API call failed: {0}")]
    ApiCallError(String),
    #[error("Missing result key: {0}")]
    MissingResultKey(String),
    #[error("Sideswap API error response: {0}")]
    ApiResponseError(String),
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
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
    #[error("Notification processing error: {0}")]
    NotificationError(String),
    #[error("Channel send error: {0}")]
    ChannelSendError(String),
    #[error("PSET parsing error: {0}")]
    PsetParsingError(String),
    #[error("Wallet signing error: {0}")]
    WalletSigningError(String),
    #[error("Connection timeout: {0}")]
    ConnectionTimeout(String),
}