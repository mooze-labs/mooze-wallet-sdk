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
    #[error("Not enough amount for chain swap operation. Requested: {0}. Minimum: {1}")]
    PegAmountTooLow(u64, u64),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("SDK error: {0}")]
    SdkError(String),
    #[error("Transaction")]
    Transaction(#[from] TransactionError),
    #[error("Context initialization failed: {context} - {reason}")]
    ContextInitializationFailed { context: String, reason: String },
    #[error("Multiple context failures: {0}")]
    MultipleContextFailures(String),
    #[error("Unsupported payment: {blockchain:?} with {payment_type}")]
    UnsupportedPayment { blockchain: crate::models::Blockchain, payment_type: String },
    #[error("Invalid address format: {address}")]
    InvalidAddress { address: String },
    #[error("All payment methods failed for amount {amount}")]
    AllPaymentMethodsFailed { amount: u64 }
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
    #[error("Insufficient funds in {context}: available {available}, required {required}")]
    InsufficientFundsDetailed { context: String, available: u64, required: u64 },
    #[error("Transaction ID validation failed: {txid}")]
    InvalidTransactionId { txid: String },
    #[error("Payment preparation failed in all contexts")]
    PaymentPreparationFailed,
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