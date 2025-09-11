use thiserror::Error;

#[derive(Debug, Error)]
pub enum SwapError {
    #[error("Invalid market.")]
    InvalidMarket,
    #[error("Quote expired.")]
    QuoteExpired,
    #[error("Dealer unavailable")]
    DealerUnavailable
}