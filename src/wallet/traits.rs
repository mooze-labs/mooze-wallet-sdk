use std::collections::HashMap;

use crate::errors::WalletError;
use crate::models::payments::*;
use crate::models::invoices::*;
use crate::models::*;

#[async_trait::async_trait]
pub trait WalletClient {
    async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError>;
    async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError>;

    async fn create_invoice(&self, amount: Option<u64>, description: Option<String>) -> Result<Invoice, WalletError>;
    async fn prepare_payment(&self, destination: &str, amount: u64, asset: Option<Asset>) -> Result<PaymentRequest, WalletError>;
    async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError>;
}