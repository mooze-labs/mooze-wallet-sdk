use std::{collections::HashMap, sync::Arc};

use anyhow::anyhow;
use breez_sdk_liquid::{
    model::{ConnectRequest, LiquidNetwork, ListPaymentsRequest, PayAmount, PayOnchainRequest, Payment, PaymentDetails, PaymentState, PreparePayOnchainRequest, PreparePayOnchainResponse, PrepareReceiveRequest, PrepareSendRequest, PrepareSendResponse, ReceiveAmount, ReceivePaymentRequest, SendPaymentRequest, SendPaymentResponse}, 
    sdk::LiquidSdk, InputType
};
use thiserror::Error;

use super::WalletClient;
use crate::errors::{TransactionError, WalletError};
use crate::models::invoices::*;
use crate::models::payments::*;
use crate::models::*;

const BREEZ_API_KEY: &str = "MIIBajCCARygAwIBAgIHPgbGnsVq8TAFBgMrZXAwEDEOMAwGA1UEAxMFQnJlZXowHhcNMjUwNDI5MDEyNDI5WhcNMzUwNDI3MDEyNDI5WjArMRMwEQYDVQQKEwpNb296ZSBMYWJzMRQwEgYDVQQDEwtMdWNjYSBHb2RveTAqMAUGAytlcAMhANCD9cvfIDwcoiDKKYdT9BunHLS2";

#[derive(Debug, Error)]
pub enum BreezError {
    #[error("SDK error: {0}")]
    SdkError(String),
    #[error("Receive amount not allowed: {0}. Min: {1} - Max: {2}")]
    LightningRecvAmountNotAllowed(u64, u64, u64),
    #[error("Send amount not allowed: {0}. Min: {1} - Max: {2}")]
    LightningSendAmountNotAllowed(u64, u64, u64),
    #[error("Transaction error: {0}")]
    TransactionError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Insufficient onchain swap amount: {0}. Required: {1}")]
    InsufficientOnchainSwapAmount(u64, u64)
}

pub struct BreezCtx {
    pub(crate) sdk: Arc<LiquidSdk>
}

impl BreezCtx {
    pub async fn new(
        mnemonic: &str,
        config: &WalletConfig
    ) -> Result<Self, anyhow::Error> {
        let network = match_liquid_network(config);
        let mut sdk_config = LiquidSdk::default_config(network, Some(BREEZ_API_KEY.to_string()))?;
        sdk_config.working_dir = config.working_dir.clone();

        let connect_req = ConnectRequest {
            config: sdk_config,
            mnemonic: Some(mnemonic.to_string()),
            passphrase: None,
            seed: None
        };

        let sdk = LiquidSdk::connect(connect_req).await?;

        Ok(BreezCtx { sdk })
    }

    pub async fn parse_address(&self, address: &str) -> Result<InputType, BreezError> {
        self.sdk.parse(address).await.map_err(|e| BreezError::InvalidAddress(e.to_string()))
    }

    pub(super) async fn balance(&self) -> Result<u64, BreezError> {
        let info = self.sdk.get_info().await.map_err(|e| BreezError::SdkError(format!("Failed to get info: {}", e)))?;
        let balance = info.wallet_info.balance_sat;

        Ok(balance)
    }
    
    pub async fn build_transaction(&self, invoice: &str, amount: u64) -> Result<PrepareSendResponse, BreezError> {
        let limits = self.sdk.fetch_lightning_limits()
            .await
            .map_err(|e| BreezError::SdkError(format!("Failed to fetch limits: {e}")))?;

        if amount < limits.send.min_sat || amount > limits.send.max_sat {
            return Err(BreezError::LightningSendAmountNotAllowed(amount, limits.send.min_sat, limits.send.max_sat));
        }

        if amount > limits.receive.max_sat || amount < limits.receive.min_sat {
            return Err(BreezError::LightningRecvAmountNotAllowed(amount, limits.receive.min_sat, limits.receive.max_sat));
        }

        let prepare_response = self.sdk.prepare_send_payment(&PrepareSendRequest {
            destination: invoice.clone().to_string(),
            amount: Some(PayAmount::Bitcoin { receiver_amount_sat: amount })
        })
        .await
        .map_err(|e| BreezError::SdkError(format!("Failed to create payment request: {e}")))?;

        Ok(prepare_response)
    }

    pub async fn build_onchain_transaction(&self, address: &str, amount: u64) -> Result<PreparePayOnchainResponse, BreezError> {
        let current_limits = self.sdk.fetch_onchain_limits()
            .await
            .map_err(|e| BreezError::SdkError(e.to_string()))?;

        if amount < current_limits.send.min_sat {
            return Err(BreezError::InsufficientOnchainSwapAmount(amount, current_limits.send.min_sat));
        }

        let prepare_response = self.sdk.prepare_pay_onchain(
            &PreparePayOnchainRequest {
                amount: PayAmount::Bitcoin { receiver_amount_sat: amount },
                fee_rate_sat_per_vbyte: None
            }
        ).await.map_err(|e| BreezError::SdkError(e.to_string()))?;

        Ok(prepare_response)
    }

    pub async fn finalize_onchain_transaction(&self, prepare_response: PreparePayOnchainResponse, address: &str) -> Result<SendPaymentResponse, BreezError> {
        let payment = self.sdk.pay_onchain(
            &PayOnchainRequest {
                prepare_response,
                recipient_address: address.to_string()
            }
        ).await.map_err(|e| BreezError::TransactionError(e.to_string()))?;

        Ok(payment)
    }

    pub async fn finalize_transaction(&self, prepare_response: PrepareSendResponse, payer_note: Option<String>) -> Result<Payment, BreezError> {
        let send_response = self.sdk.send_payment(
            &SendPaymentRequest {
                prepare_response,
                use_asset_fees: None,
                payer_note
            }
        )
        .await
        .map_err(|e| BreezError::TransactionError(format!("Failed to send payment: {e}")))?;

        let payment = send_response.payment;

        Ok(payment)
    }

}

#[async_trait::async_trait]
impl WalletClient for BreezCtx {
    async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError> {
        let mut balances: HashMap<Asset, u64> = HashMap::new();
        let info = self.sdk.get_info()
            .await
            .map_err(|e| WalletError::ConnectionError(e.to_string()))?;
        let balance = info.wallet_info.balance_sat;

        balances.insert(Asset::BitcoinLayer2, balance);

        Ok(balances)
    }

    async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError> {
        let payments = self.sdk
            .list_payments(&ListPaymentsRequest::default())
            .await
            .map_err(|e| WalletError::SdkError(e.to_string()))?
            .into_iter()
            .filter_map(|p| payment_into_wallet_transaction(&p).ok())
            .collect();

        Ok(payments)
    }

    async fn create_invoice(&self, amount: Option<u64>, description: Option<String>) -> Result<Invoice, WalletError> {
        let recv_amount = Some(ReceiveAmount::Bitcoin { 
            payer_amount_sat: amount.ok_or(TransactionError::InvoiceAmountRequired)? 
        });

        let prepare_response = self.sdk.prepare_receive_payment(
            &PrepareReceiveRequest {
                payment_method: breez_sdk_liquid::model::PaymentMethod::Bolt11Invoice,
                amount: recv_amount
            }
        )
        .await
        .map_err(|e| WalletError::SdkError(e.to_string()))?;

        let fee = prepare_response.fees_sat;
        let payment = self.sdk.receive_payment(
            &ReceivePaymentRequest {
                prepare_response,
                description: description.clone(),
                use_description_hash: None,
                payer_note: None
            }
        )
        .await
        .map_err(|e| WalletError::ConnectionError(e.to_string()))?;

        let invoice = Invoice {
            address: payment.destination.clone(),
            invoice_type: InvoiceType::Lightning(payment),
            amount,
            description,
            fee: Some(fee)
        };

        Ok(invoice)
    }

    async fn prepare_payment(&self, destination: &str, amount: u64, asset: Option<Asset>) -> Result<PaymentRequest, WalletError> {
        let swap_limits = self.sdk.fetch_lightning_limits()
            .await
            .map_err(|e| WalletError::ConnectionError(format!("Failed to retrieve Lightning swap amounts: {e}")))?;

        if (amount > swap_limits.send.max_sat) || (amount < swap_limits.send.min_sat) {
            return Err(TransactionError::InvalidSwapAmount(amount))?
        }

        let prepare_response = self.sdk.prepare_send_payment(
            &PrepareSendRequest { 
                destination: destination.to_string(), 
                amount: Some(PayAmount::Bitcoin { receiver_amount_sat: amount }) 
            }
        ).await.map_err(|e| WalletError::SdkError(e.to_string()))?;

        let payment_req = PaymentRequest::new(
            destination,
            amount,
            Asset::BitcoinLayer2,
            prepare_response.fees_sat.unwrap_or(0_u64),
            Blockchain::Lightning,
            PreparedPayment::Lightning(prepare_response)
        );

        Ok(payment_req)
    }

    async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError> {
        if let PreparedPayment::Lightning(prepare_response) = &payment_request.prepared_payment {
            let send_response = self.sdk
                .send_payment(&SendPaymentRequest { 
                    prepare_response: prepare_response.to_owned(), 
                    use_asset_fees: None, 
                    payer_note: None 
                }).await
                .map_err(|e| WalletError::SdkError(e.to_string()))?;

            let swap_id = match &send_response.payment.details {
                PaymentDetails::Lightning { swap_id, .. } => Ok(swap_id),
                PaymentDetails::Bitcoin { swap_id, .. } => Ok(swap_id),
                _ => Err(TransactionError::SwapIdUnavailable)
            }?;

            return Ok(send_response.payment.tx_id.unwrap_or(swap_id.clone()));
        }

        Err(TransactionError::PaymentTypeDoesNotMatchNetwork)?
    }
}

pub async fn prepare_onchain_payment(sdk: Arc<LiquidSdk>, amount: u64) -> Result<PreparePayOnchainResponse, BreezError> {
    let current_limits = sdk.fetch_onchain_limits()
        .await
        .map_err(|e| BreezError::SdkError(e.to_string()))?;

    if amount < current_limits.send.min_sat {
        return Err(BreezError::InsufficientOnchainSwapAmount(amount, current_limits.send.min_sat));
    }

    let prepare_response = sdk.prepare_pay_onchain(
        &PreparePayOnchainRequest {
            amount: PayAmount::Bitcoin { receiver_amount_sat: amount },
            fee_rate_sat_per_vbyte: None
        }
    ).await.map_err(|e| BreezError::SdkError(e.to_string()))?;

    Ok(prepare_response)
}

pub async fn pay_onchain(sdk: Arc<LiquidSdk>, prepare_response: PreparePayOnchainResponse, address: &str) -> Result<SendPaymentResponse, BreezError> {
    let payment = sdk.pay_onchain(
        &PayOnchainRequest { address: address.to_string(), prepare_response }
    ).await.map_err(|e| BreezError::SdkError(e.to_string()))?;

    Ok(payment)
}

fn payment_into_wallet_transaction(payment: &Payment) -> Result<WalletTransaction, WalletError> {
    let txid = match &payment.tx_id {
        Some(txid) => txid,
        None => match &payment.details {
            PaymentDetails::Lightning { swap_id, .. } => swap_id,
            PaymentDetails::Bitcoin { swap_id, .. } => swap_id,
            _ => return Err(TransactionError::SwapIdUnavailable)? // Liquid transactions must be done through LwkCtx
        }
    };

    let direction = match payment.payment_type {
        breez_sdk_liquid::model::PaymentType::Receive => TransactionDirection::Incoming,
        breez_sdk_liquid::model::PaymentType::Send => TransactionDirection::Outgoing
    };

    let blockchain = match payment.details {
        PaymentDetails::Lightning { .. } => Blockchain::Lightning,
        PaymentDetails::Liquid { .. } => Blockchain::Liquid,
        PaymentDetails::Bitcoin { .. } => Blockchain::Bitcoin
    };

    Ok(WalletTransaction {
        txid: txid.clone(),
        amount: payment.amount_sat,
        asset: Asset::BitcoinLayer2,
        fee: Some(payment.fees_sat),
        timestamp: Some(payment.timestamp.into()),
        confirmed: (payment.status == PaymentState::Complete),
        direction: direction,
        blockchain
    })

}

fn match_liquid_network(config: &WalletConfig) -> LiquidNetwork {
    match config.network_type {
        NetworkType::Mainnet => LiquidNetwork::Mainnet,
        NetworkType::Testnet => LiquidNetwork::Testnet,
        NetworkType::Regtest => LiquidNetwork::Regtest
    }
}