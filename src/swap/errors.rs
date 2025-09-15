use thiserror::Error;

use crate::{models::Asset, swap::api};

#[derive(Debug, Error)]
pub enum SwapError {
    #[error("Invalid market.")]
    InvalidMarket,
    #[error("Quote expired.")]
    QuoteExpired,
    #[error("Dealer unavailable")]
    DealerUnavailable,
    #[error("Connection error")]
    ConnectionError,
    #[error("Context error: {0}")]
    ContextError(String),
    #[error("Insufficient amount.")]
    InsufficientFunds,
    #[error("Sideswap error: {0}")]
    SideswapError(#[from] api::sideswap::SideswapError),
    #[error("Invalid asset")]
    InvalidAsset
}