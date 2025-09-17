use thiserror::Error;

use crate::{errors::WalletError, models::Asset, swap::api};

#[derive(Debug, Error)]
pub enum SwapError {
    #[error("Sideswap error: {0}")]
    SideswapError(#[from] api::sideswap::SideswapError),
    #[error("Wallet error: {0}")]
    WalletError(#[from] WalletError),
    #[error("Argument error: {0}")]
    ArgumentError(String),
    #[error("Context error: {0}")]
    ContextError(String),
    #[error("Invalid quote")]
    InvalidQuote,
    #[error("Invalid market.")]
    InvalidMarket,
    #[error("Quote expired.")]
    QuoteExpired,
    #[error("Dealer unavailable")]
    DealerUnavailable,
    #[error("Connection error")]
    ConnectionError,
    #[error("Insufficient amount.")]
    InsufficientFunds,
    #[error("Invalid asset")]
    InvalidAsset,
    #[error("Amount below min peg-out limit.")]
    AmountBelowMinPegOut,
    #[error("Amount below minimum: {0}")]
    AmountBelowMinimum(u64),
    #[error("Amount above maximum: {0}")]
    AmountAboveMax(u64),
}