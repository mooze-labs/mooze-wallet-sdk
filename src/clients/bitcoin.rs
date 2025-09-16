use std::{collections::HashMap, str::FromStr, sync::Arc};

use bdk_wallet::{
    bitcoin::{bip32::Xpriv, Amount, Psbt}, 
    keys::{bip39::Mnemonic, DescriptorPublicKey}, 
    miniscript::Descriptor, 
    template::{Bip84, DescriptorTemplate}, 
    SignOptions, 
    Update
};
use bdk_electrum::{
    electrum_client::{self, Client},
    BdkElectrumClient
};

use parking_lot::RwLock;
use thiserror::Error;

use super::WalletClient;
use crate::errors::{TransactionError, WalletError};
use crate::models::invoices::*;
use crate::models::payments::*;
use crate::models::*;

#[derive(Debug, Error)]
pub enum BdkError {
    #[error("Descriptor error: {0}")]
    DescriptorError(String),
    #[error("Electrum error: {0}")]
    ElectrumError(String),
    #[error("Format error: {0}")]
    FormatError(String),
    #[error("Mnemonic error: {0}")]
    MnemonicError(String),
    #[error("Signature error: {0}")] 
    SignatureError(String),
    #[error("Wallet error: {0}")]
    WalletError(String),
}

pub struct BitcoinCtx {
    config: WalletConfig,
    electrum_client: BdkElectrumClient<Client>,
    pub(crate) wallet: Arc<RwLock<bdk_wallet::Wallet>>,
}

impl BitcoinCtx {
    pub fn new(
        mnemonic: &str,
        config: &WalletConfig,
    ) -> Result<Self, BdkError> {
        let mnemonic = Mnemonic::parse(mnemonic)
            .map_err(|e| BdkError::MnemonicError(format!("Failed to parse mnemonic: {}", e)))?;

        let network = match_bitcoin_network(&config);
        let seed = mnemonic.to_seed("");

        let (descriptor, change_descriptor) = generate_wallet_descriptors(&seed, &network)?;

        let wallet = bdk_wallet::Wallet::create(descriptor, change_descriptor)
            .network(network)
            .create_wallet_no_persist()
            .map_err(|e| BdkError::WalletError(format!("Failed to create wallet: {}", e)))?;

        let electrum_client: BdkElectrumClient<Client> = BdkElectrumClient::new(
            electrum_client::Client::new(&config.bitcoin_electrum_url)
                .map_err(|e| BdkError::WalletError(format!("Failed to create Electrum client: {}", e)))?
        );

        Ok(BitcoinCtx { 
            config: config.clone(), 
            electrum_client, 
            wallet: Arc::new(RwLock::new(wallet)) 
        } )
    }

    pub fn full_scan(&self) -> Result<(), BdkError> {
        let full_scan_req = self.wallet.read().start_full_scan(); 
        let update = self.electrum_client.full_scan(
            full_scan_req,
            self.config.stop_gap,
            self.config.batch_size,
            true
        ).map_err(|e| BdkError::ElectrumError(format!("Failed to scan wallet: {e}")))?;

        self.apply_update(update)?;

        Ok(())
    }

    pub fn sync(&self) -> Result<(), BdkError> {
        let sync_req = self.wallet.read().start_sync_with_revealed_spks();
        let sync_response = self.electrum_client.sync(sync_req, self.config.batch_size, true)
            .map_err(|e| BdkError::ElectrumError(format!("Failed to sync wallet to new state: {e}")))?;

        if sync_response.is_empty() {
            return Ok(());
        }

        self.apply_update(sync_response)?;

        Ok(())
    }

    fn build_transaction(&self, address: &str, satoshi: u64) -> Result<Psbt, BdkError> {
        let mut wallet = self.wallet.write();
        let network = wallet.network();

        let address = bdk_wallet::bitcoin::Address::from_str(address)
            .map_err(|e| BdkError::FormatError(format!("Invalid address: {}", e)))?
            .require_network(network)
            .map_err(|e| BdkError::FormatError(format!("Address network mismatch: {}", e)))?;

        let psbt = {
            let mut builder = wallet.build_tx();
            builder.ordering(bdk_wallet::TxOrdering::Untouched);
            builder.add_recipient(address.script_pubkey(), Amount::from_sat(satoshi));
            builder.finish()
                .map_err(|e| BdkError::WalletError(format!("Failed to build transaction: {}", e)))?
        };

        Ok(psbt)
    }

    fn sign_transaction(&self, psbt: &mut Psbt) -> Result<(), BdkError> {
        let wallet = self.wallet.read();

        let signed = wallet.sign(psbt, SignOptions::default())
            .map_err(|e| BdkError::SignatureError(format!("Failed to sign transaction: {}", e)))?;

        if (!signed) {
            return Err(BdkError::SignatureError("Transaction not fully signed".to_string()));
        }

        let finalized = wallet.finalize_psbt(psbt, SignOptions::default())
            .map_err(|e| BdkError::SignatureError(format!("Failed to finalize PSBT: {}", e)))?;

        if (!finalized) {
            return Err(BdkError::SignatureError("PSBT not fully finalized".to_string()));
        }

        Ok(())
    }

    fn broadcast_transaction(&self, psbt: Psbt) -> Result<String, BdkError> {
        let tx = psbt.extract_tx()
                .map_err(|e| BdkError::FormatError(format!("Failed to extract transaction: {}", e)))?;
        let txid = self.electrum_client.transaction_broadcast(&tx).map_err(|e| BdkError::WalletError(format!("Failed to broadcast transaction: {}", e)))?;

        Ok(txid.to_string())
    }

    fn apply_update(&self, update: impl Into<Update>) -> Result<(), BdkError> {
        self.wallet.write()
            .apply_update(update)
            .map_err(|e| BdkError::WalletError(format!("Failed to update wallet: {e}")))
    }
}

#[async_trait::async_trait]
impl WalletClient for BitcoinCtx {
    async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError> {
        let mut balance = HashMap::new();
        let trusted_spendable = self.wallet.read().balance().trusted_spendable().to_sat();

        balance.insert(Asset::BitcoinOnchain, trusted_spendable);
        Ok(balance)
    }

    async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError> {
        let wallet = self.wallet.read();
        let transactions = wallet
            .transactions()
            .filter_map(|i| wallet.tx_details(i.tx_node.txid))
            .filter_map(|tx_details| tx_details_into_wallet_transaction(&tx_details).ok())
            .collect();

        Ok(transactions)
    }

    async fn create_invoice(&self, _amount: Option<u64>, _description: Option<String>) -> Result<Invoice, WalletError> { 
        let mut wallet = self.wallet.write();
        let next_address = wallet.next_unused_address(bdk_wallet::KeychainKind::External);

        let invoice = Invoice {
            address: next_address.address.to_string(),
            invoice_type: InvoiceType::Onchain,
            amount: None,
            description: None,
            fee: None
        };

        Ok(invoice)
    }

    async fn prepare_payment(&self, destination: &str, amount: u64, _asset: Option<Asset>) -> Result<PaymentRequest, WalletError> {
        if self.wallet.read().balance().trusted_spendable().to_sat() < amount {
            return Err(TransactionError::InsufficientFunds)?
        }

        let psbt = self.build_transaction(destination, amount)?;
        let fees = psbt.fee()
            .map_err(|e| WalletError::SdkError(e.to_string()))?
            .to_sat();

        let payment_request = PaymentRequest::new(
            destination,
            amount,
            Asset::BitcoinOnchain,
            fees,
            Blockchain::Bitcoin,
            PreparedPayment::Onchain(psbt)
        );

        Ok(payment_request)
    }

    async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError> {
        if let PreparedPayment::Onchain(psbt) = &payment_request.prepared_payment {
            let mut psbt = psbt.clone();
            self.sign_transaction(&mut psbt)?;
            let txid = self.broadcast_transaction(psbt)?;

            return Ok(txid);
        }

        Err(TransactionError::PaymentTypeDoesNotMatchNetwork)?
    }
}

fn generate_wallet_descriptors(seed: &[u8; 64], network: &bdk_wallet::bitcoin::Network) -> Result<(Descriptor<DescriptorPublicKey>, Descriptor<DescriptorPublicKey>), BdkError> {
    let xprv: Xpriv = Xpriv::new_master(network.clone(), seed).map_err(|e| BdkError::DescriptorError(format!("Failed to create xprv: {}", e)))?;

    let (descriptor, _, _) = Bip84(xprv, bdk_wallet::KeychainKind::External)
        .build(network.clone())
        .map_err(|e| BdkError::DescriptorError(format!("Failed to create descriptor: {}", e)))?;
    let (change_descriptor, _, _) = Bip84(xprv, bdk_wallet::KeychainKind::Internal)
        .build(network.clone())
        .map_err(|e| BdkError::DescriptorError(format!("Failed to create change descriptor: {}", e)))?;

    Ok((descriptor, change_descriptor))
}

fn match_bitcoin_network(config: &WalletConfig) -> bdk_wallet::bitcoin::Network {
    match config.network_type {
        NetworkType::Mainnet => bdk_wallet::bitcoin::Network::Bitcoin,
        NetworkType::Testnet => bdk_wallet::bitcoin::Network::Testnet4,
        NetworkType::Regtest => bdk_wallet::bitcoin::Network::Regtest,
    }
}

fn tx_details_into_wallet_transaction(tx_details: &bdk_wallet::TxDetails) -> Result<WalletTransaction, WalletError> {
    let timestamp = match tx_details.chain_position {
        bdk_wallet::chain::ChainPosition::Confirmed { anchor, .. } => Some(anchor.confirmation_time),
        bdk_wallet::chain::ChainPosition::Unconfirmed { last_seen, .. } => last_seen
    };

    // TODO: Handle cases where the value is out of range
    let amount = tx_details.balance_delta.to_unsigned()
        .map_err(|e| WalletError::SdkError(e.to_string()))?
        .to_sat();

    let wallet_tx = WalletTransaction {
        txid: tx_details.txid.to_string(),
        amount,
        asset: Asset::BitcoinOnchain,
        fee: tx_details.fee.and_then(|f| Some(f.to_sat())),
        timestamp: timestamp, 
        confirmed: tx_details.chain_position.is_confirmed(),
        direction: (if (tx_details.balance_delta.to_sat() > 0) { TransactionDirection::Incoming } else { TransactionDirection::Outgoing }),
        blockchain: Blockchain::Bitcoin
    };

    Ok(wallet_tx)
}