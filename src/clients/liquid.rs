use std::{collections::HashMap, str::FromStr, sync::Arc};

use lwk_common::Signer;
use lwk_signer::SwSigner;
use lwk_wollet::{
    blocking::BlockchainBackend,
    elements::{pset::PartiallySignedTransaction, AssetId, OutPoint}, 
    full_scan_with_electrum_client, 
    ElectrumClient, 
    ElectrumUrl, 
    ElementsNetwork, 
    NoPersist, 
    UnvalidatedRecipient, 
    WalletTx, 
    Wollet, 
    WolletDescriptor
};
use thiserror::Error;
use parking_lot::{Mutex, RwLock};

use crate::models::{NetworkType, WalletConfig};


use super::WalletClient;
use crate::errors::{TransactionError, WalletError};
use crate::models::invoices::*;
use crate::models::payments::*;
use crate::models::*;

const ELECTRUM_URL: &str = "ssl://electrum.blockstream.info:50002";
const REGTEST_POLICY_ASSET: &str = "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225";

#[derive(Debug, Error)]
pub enum LwkError {
    #[error("Electrum error: {0}")]
    ElectrumError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Transaction error: {0}")]
    TransactionError(String),
    #[error("Wallet error: {0}")]
    WalletError(String),
}

pub struct LwkCtx {
    electrum_client: Arc<Mutex<ElectrumClient>>,
    policy_asset: String,
    pub(crate) signer: SwSigner,
    pub(crate) wollet: Arc<RwLock<Wollet>>,
}

impl LwkCtx {
    pub fn new(mnemonic: &str, config: &WalletConfig) -> Result<Self, anyhow::Error> {
        let network = match_elements_network(config);
        let policy_asset = match network {
            ElementsNetwork::Liquid => String::from("6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d"),
            ElementsNetwork::LiquidTestnet => String::from("6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d"),
            _ => String::from(REGTEST_POLICY_ASSET)
        };
        let signer = SwSigner::new(mnemonic, network == ElementsNetwork::Liquid)?;

        let descriptor: WolletDescriptor = lwk_common::singlesig_desc(&signer, lwk_common::Singlesig::Wpkh, lwk_common::DescriptorBlindingKey::Slip77)
            .map_err(|e| anyhow::anyhow!("Failed to create descriptor: {}", e))?
            .parse()?;


        let electrum_url = ElectrumUrl::new(ELECTRUM_URL, true, true)?;
        let mut electrum_client = ElectrumClient::new(&electrum_url)?;
        let mut wallet = Wollet::new(network, NoPersist::new(), descriptor)?;

        let _ = full_scan_with_electrum_client(&mut wallet, &mut electrum_client)?;

        Ok(LwkCtx {
            electrum_client: Arc::new(Mutex::new(electrum_client)),
            policy_asset,
            signer,
            wollet: Arc::new(RwLock::new(wallet)),
        })
    }
    
    fn build_transaction(&self, address: &str, satoshi: u64, asset: &str) -> Result<PartiallySignedTransaction, LwkError> {
        let wollet = self.wollet.read();

        let pset = wollet.tx_builder()
            .add_unvalidated_recipient(&UnvalidatedRecipient { satoshi, address: address.to_string(), asset: asset.to_string() })
            .map_err(|e| LwkError::TransactionError(format!("Failed to add recipient: {}", e)))?
            .enable_ct_discount()
            .finish()
            .map_err(|e| LwkError::TransactionError(format!("Failed to finish transaction: {}", e)))?;

        Ok(pset)
    }

    fn sign_transaction(&self, pset: &PartiallySignedTransaction) -> Result<PartiallySignedTransaction, LwkError> {
        let mut pset = pset.clone();
        let _signed_pset = self.signer.sign(&mut pset).map_err(|e| LwkError::TransactionError(format!("Failed to sign transaction: {}", e)))?;

        Ok(pset)
    }

    fn broadcast_transaction(&self, pset: &PartiallySignedTransaction) -> Result<String, LwkError> {
        let mut pset = pset.clone();
        let (wollet, electrum_client) = (self.wollet.read(), self.electrum_client.lock());
        let tx = wollet.finalize(&mut pset)
            .map_err(|e| LwkError::TransactionError(format!("Failed to finalize transaction: {}", e)))?;

        let txid = electrum_client.broadcast(&tx).map_err(|e| LwkError::ElectrumError(format!("Failed to broadcast transaction: {}", e)))?;
        Ok(txid.to_string())
    }

    pub fn generate_new_address(&self) -> Result<String, LwkError> {
        let wollet = self.wollet.read();
        let address = wollet.address(None).map_err(|e| LwkError::WalletError(format!("Failed to generate new address: {}", e)))?;

        Ok(address.address().to_string())
    }

    pub(crate) fn sync(&self) -> Result<(), LwkError> {
        let wollet = self.wollet.write();
        let mut electrum_client = self.electrum_client.lock();

        let update = electrum_client.full_scan(& *wollet).map_err(|e| LwkError::ElectrumError(format!("Failed to sync wallet: {}", e)))?;

        match update {
            Some(update) => self.apply_update(update),
            None => Ok(()),
        }
    }

    /// Signs a PSET that contains UTXOs owned by different parties e.g. my wallet and another wallet.
    /// It goes through the UTXOs, adds the wallet's details and returns it to be used on coinjoin operations.
    pub(crate) fn sign_with_extra_details(&self, pset: &PartiallySignedTransaction) -> Result<PartiallySignedTransaction, LwkError> {
        let wollet = self.wollet.write();
        let mut signed_pset_1 = self.sign_transaction(pset)?;

        for input in signed_pset_1.inputs_mut().iter_mut() {
            let outpoint = OutPoint {
                txid: input.previous_txid,
                vout: input.previous_output_index,
            };
            let tx = wollet
                .transaction(&outpoint.txid)
                .map_err(|e| {
                    LwkError::TransactionError(format!(
                        "Failed to get transaction output: {}",
                        e.to_string()
                    ))
                })?
                .ok_or_else(|| {
                    LwkError::TransactionError("Transaction output not found".to_string())
                })?;
            let tx_out = tx
                .tx
                .output
                .get(outpoint.vout as usize)
                .ok_or(LwkError::TransactionError("Output not found".to_string()))?;

            input.in_utxo_rangeproof = tx_out.witness.rangeproof.clone();
            input.witness_utxo = Some(tx_out.clone());
        }

        wollet.add_details(&mut signed_pset_1).map_err(|e| {
            LwkError::TransactionError(format!("Failed to add details: {}", e.to_string()))
        })?;
        let mut signed_pset_2 = self.sign_transaction(&signed_pset_1)?;

        for input in signed_pset_2.inputs_mut() {
            if let Some((public_key, input_sign)) = input.partial_sigs.iter().next() {
                input.final_script_witness = Some(vec![input_sign.clone(), public_key.to_bytes()]);
            }
        }

        Ok(signed_pset_2)
    }

    fn apply_update(&self, update: lwk_wollet::Update) -> Result<(), LwkError> {
        let mut wollet = self.wollet.write();
        let _ = wollet.apply_update_no_persist(update).map_err(|e| LwkError::WalletError(format!("Failed to apply update: {}", e)))?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl WalletClient for LwkCtx {
    async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError> {
        let wallet_balances = self.wollet.read()
            .balance()
            .map_err(|e| WalletError::SdkError(format!("Failed to get balances: {e}")))?;

        let balances: HashMap<Asset, u64> = wallet_balances.iter()
            .map(|(k, v)| {
                let asset = if *k == lwk_wollet::elements::AssetId::LIQUID_BTC { 
                    Asset::BitcoinLayer2 
                } else {
                    Asset::LiquidAsset(k.clone().to_string())
                };

                (asset, *v)
            }).collect();
        
        Ok(balances)
    }

    async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError> {
        let wallet = self.wollet.read();
        let transactions = wallet.transactions()
            .map_err(|e| WalletError::SdkError(e.to_string()))?
            .into_iter()
            .map(|tx| transaction_to_wallet_tx(&tx))
            .filter_map(|tx| tx.ok())
            .collect();

        Ok(transactions)
    }

    async fn create_invoice(&self, _amount: Option<u64>, _description: Option<String>) -> Result<Invoice, WalletError> {
        let address = self.wollet.read()
            .address(None)
            .map_err(|e| WalletError::SdkError(format!("Failed to generate address: {e}")))?;

        let invoice = Invoice {
            address: address.address().to_string(),
            invoice_type: InvoiceType::Liquid,
            amount: None,
            description: None,
            fee: None
        };

        Ok(invoice)
    }

    async fn prepare_payment(&self, destination: &str, amount: u64, asset: Option<Asset>) -> Result<PaymentRequest, WalletError> {
        let pset = match &asset {
            None => self.build_transaction(destination, amount, &self.policy_asset),
            Some(asset) => match asset {
                Asset::BitcoinLayer2 => self.build_transaction(destination, amount, &self.policy_asset),
                Asset::LiquidAsset(asset_id) => self.build_transaction(destination, amount, &asset_id),
                _ => { return Err(TransactionError::PaymentTypeDoesNotMatchNetwork)? }
            }
        }?;

        let fees = pset.extract_tx()
            .map_err(|e| WalletError::SdkError(e.to_string()))?
            .fee_in(lwk_wollet::elements::AssetId::LIQUID_BTC);

        let payment_request = PaymentRequest::new(
            destination,
            amount,
            asset.unwrap_or(Asset::BitcoinLayer2),
            fees,
            Blockchain::Liquid,
            PreparedPayment::Liquid(pset)
        );

        Ok(payment_request)
    }

    async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError> {
        if let PreparedPayment::Liquid(pset) = &payment_request.prepared_payment {
            let signed_pset = self.sign_transaction(&pset)?;
            let txid = self.broadcast_transaction(&signed_pset)?;

            return Ok(txid)
        }

        Err(TransactionError::PaymentTypeDoesNotMatchNetwork)?
    }
}

fn transaction_to_wallet_tx(tx: &WalletTx) -> Result<WalletTransaction, TransactionError> {
    // See if it is an asset transfer first
    let maybe_asset_balance = tx.balance
        .iter()
        .find(|(asset_id, _)| asset_id.clone().ne(&lwk_wollet::elements::AssetId::LIQUID_BTC))
        .map(|(key, value)| (*key, *value));

    let net_amount= match maybe_asset_balance {
        Some(balance) => balance.1,
        None => {
            let lbtc_balance= tx.balance.get(&lwk_wollet::elements::AssetId::LIQUID_BTC);
            match lbtc_balance {
                Some(balance) => *balance,
                None => return Err(TransactionError::BalanceUnavailable)
            }
        }
    };

    let direction = if net_amount > 0 {
        TransactionDirection::Incoming
    } else {
        TransactionDirection::Outgoing
    };

    let asset = match maybe_asset_balance {
        Some(balance) => Asset::LiquidAsset(balance.0.to_string()),
        None => Asset::BitcoinLayer2
    };

    Ok(WalletTransaction {
        txid: tx.txid.to_string(),
        amount: (net_amount.abs() as u64),
        asset,
        fee: Some(tx.fee),
        timestamp: tx.timestamp.and_then(|t| Some(t as u64)),
        confirmed: tx.height.is_some(),
        direction,
        blockchain: Blockchain::Liquid
    })
}

fn match_elements_network(config: &WalletConfig) -> ElementsNetwork {
    match config.network_type {
        NetworkType::Mainnet => ElementsNetwork::Liquid,
        NetworkType::Testnet => ElementsNetwork::LiquidTestnet,
        NetworkType::Regtest => ElementsNetwork::ElementsRegtest { policy_asset: AssetId::from_str(&config.liquid_policy_asset).unwrap() },
    }
}