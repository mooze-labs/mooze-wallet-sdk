use thiserror::Error;

use crate::clients::bitcoin::BdkError;
use crate::clients::liquid::LwkError;
use crate::clients::breez::BreezError;

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("BDK error: {0}")]
    BitcoinError(#[from] BdkError),
    #[error("Breez error: {0}")]
    BreezClientError(#[from] BreezError),
    #[error("LWK error: {0}")]
    LiquidError(#[from] LwkError),
    #[error("Failed to retrieve balance: {0}")]
    BalanceUnavailable(String),
    #[error("Not enough amount for chain swap operation. Requested: {0}. Minimum: {0}")]
    PegAmountTooLow(u64, u64),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("SDK error: {0}")]
    SdkError(String),
    #[error("Transaction")]
    Transaction(#[from] TransactionError)
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("Balance unavailable.")]
    BalanceUnavailable,
    #[error("TXID not found.")]
    TxidNotFound,
    #[error("Swap ID unavailable")]
    SwapIdUnavailable,
    #[error("Invalid swap amount: {0}")]
    InvalidSwapAmount(u64),
    #[error("Payment type does not match network")]
    PaymentTypeDoesNotMatchNetwork,
    #[error("Amount required for invoice.")]
    InvoiceAmountRequired,
    #[error("Insufficient funds.")]
    InsufficientFunds,
}

#[derive(Debug, Error)]
pub enum SwapError {
    #[error("Invalid market.")]
    InvalidMarket,
    #[error("Quote expired.")]
    QuoteExpired,
    #[error("Dealer unavailable")]
    DealerUnavailable
}