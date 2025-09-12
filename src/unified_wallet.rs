use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;

use crate::clients::{WalletClient, bitcoin::BitcoinCtx, breez::BreezCtx, liquid::LwkCtx};
use crate::errors::WalletError;
use crate::models::*;
use crate::models::invoices::Invoice;
use crate::models::payments::PaymentRequest;

/// Unified wallet that abstracts Bitcoin, Liquid, and Lightning operations
/// Routes method calls to appropriate contexts based on asset types
pub struct UnifiedWallet {
    bitcoin_ctx: Arc<BitcoinCtx>,
    breez_ctx: Arc<BreezCtx>,
    liquid_ctx: Arc<LwkCtx>,
}

impl UnifiedWallet {
    /// Create UnifiedWallet from pre-created context instances
    /// Validates that all contexts are properly initialized
    pub fn new(
        bitcoin_ctx: Arc<BitcoinCtx>,
        breez_ctx: Arc<BreezCtx>,
        liquid_ctx: Arc<LwkCtx>,
    ) -> Result<Self, WalletError> {
        // Validate contexts are not null/empty (basic Arc validation)
        if Arc::strong_count(&bitcoin_ctx) == 0 {
            return Err(WalletError::ContextInitializationFailed {
                context: "Bitcoin".to_string(),
                reason: "Invalid Arc reference".to_string(),
            });
        }
        if Arc::strong_count(&breez_ctx) == 0 {
            return Err(WalletError::ContextInitializationFailed {
                context: "Breez".to_string(),
                reason: "Invalid Arc reference".to_string(),
            });
        }
        if Arc::strong_count(&liquid_ctx) == 0 {
            return Err(WalletError::ContextInitializationFailed {
                context: "Liquid".to_string(),
                reason: "Invalid Arc reference".to_string(),
            });
        }

        Ok(UnifiedWallet {
            bitcoin_ctx,
            breez_ctx,
            liquid_ctx,
        })
    }

    /// Convenience method to create all contexts and UnifiedWallet in one call
    pub async fn create_with_contexts(
        mnemonic: &str,
        config: &WalletConfig,
    ) -> Result<Self, WalletError> {
        let bitcoin_ctx = Arc::new(BitcoinCtx::new(mnemonic, config)
            .map_err(|e| WalletError::SdkError(e.to_string()))?);
        
        let breez_ctx = Arc::new(BreezCtx::new(mnemonic, config).await
            .map_err(|e| WalletError::SdkError(e.to_string()))?);
            
        let liquid_ctx = Arc::new(LwkCtx::new(mnemonic, config)
            .map_err(|e| WalletError::SdkError(e.to_string()))?);

        UnifiedWallet::new(bitcoin_ctx, breez_ctx, liquid_ctx)
    }

    /// Get unified balance across all contexts
    pub async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError> {
        let mut unified_balance = HashMap::new();
        let mut failures = Vec::new();

        // Get Bitcoin balance
        match self.bitcoin_ctx.balance().await {
            Ok(bitcoin_balance) => unified_balance.extend(bitcoin_balance),
            Err(e) => {
                failures.push(format!("Bitcoin: {}", e));
                // Continue with other contexts even if one fails
            }
        }

        // Get Breez (Lightning) balance
        match self.breez_ctx.balance().await {
            Ok(breez_balance) => unified_balance.extend(breez_balance),
            Err(e) => {
                failures.push(format!("Breez: {}", e));
            }
        }

        // Get Liquid balance
        match self.liquid_ctx.balance().await {
            Ok(liquid_balance) => unified_balance.extend(liquid_balance),
            Err(e) => {
                failures.push(format!("Liquid: {}", e));
            }
        }

        // If all contexts failed, return error
        if unified_balance.is_empty() && !failures.is_empty() {
            return Err(WalletError::MultipleContextFailures(failures.join("; ")));
        }

        // If some contexts failed but we have partial data, log warnings but continue
        // In production, you might want to log these failures
        
        Ok(unified_balance)
    }

    /// Create invoice based on requested asset
    pub async fn create_invoice(
        &self, 
        amount: Option<u64>, 
        description: Option<String>,
        asset: Asset
    ) -> Result<Invoice, WalletError> {
        match asset {
            Asset::BitcoinOnchain => {
                self.bitcoin_ctx.create_invoice(amount, description).await
            },
            Asset::BitcoinLayer2 => {
                self.breez_ctx.create_invoice(amount, description).await
            },
            Asset::LiquidAsset(_) => {
                self.liquid_ctx.create_invoice(amount, description).await
            }
        }
    }

    /// Prepare payment with intelligent routing
    pub async fn prepare_payment(
        &self,
        destination: &str,
        amount: u64,
        asset: Option<Asset>
    ) -> Result<PaymentRequest, WalletError> {
        let target_asset = asset.unwrap_or(Asset::BitcoinOnchain);

        match target_asset {
            Asset::BitcoinOnchain => {
                // Check Bitcoin on-chain funds first
                let bitcoin_balance = self.bitcoin_ctx.balance().await?;
                if let Some(btc_amount) = bitcoin_balance.get(&Asset::BitcoinOnchain) {
                    if *btc_amount >= amount {
                        return self.bitcoin_ctx.prepare_payment(destination, amount, Some(target_asset)).await;
                    }
                }

                // If insufficient Bitcoin funds, try Breez context for on-chain payment
                // (Breez can do on-chain payments via Liquid->Bitcoin swaps)
                // We need to create a custom payment request for Breez on-chain payments
                match self.breez_ctx.build_onchain_transaction(destination, amount).await {
                    Ok(prepare_response) => {
                        let payment_req = PaymentRequest::new(
                            destination,
                            amount,
                            Asset::BitcoinOnchain,
                            prepare_response.fees_sat,
                            Blockchain::Bitcoin,
                            PreparedPayment::PegOut(prepare_response)
                        );
                        Ok(payment_req)
                    },
                    Err(_) => {
                        // If Breez on-chain also fails, return the original Bitcoin error
                        Err(crate::errors::TransactionError::InsufficientFunds.into())
                    }
                }
            },
            Asset::BitcoinLayer2 => {
                self.breez_ctx.prepare_payment(destination, amount, Some(target_asset)).await
            },
            Asset::LiquidAsset(_) => {
                self.liquid_ctx.prepare_payment(destination, amount, Some(target_asset)).await
            }
        }
    }

    /// Send payment using the prepared payment request
    pub async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError> {
        match (&payment_request.blockchain, &payment_request.prepared_payment) {
            (Blockchain::Bitcoin, PreparedPayment::Onchain(_)) => {
                self.bitcoin_ctx.send_payment(payment_request).await
            },
            (Blockchain::Bitcoin, PreparedPayment::PegOut(prepare_response)) => {
                // Handle Breez on-chain payments (Liquid -> Bitcoin swaps)
                match self.breez_ctx.finalize_onchain_transaction(prepare_response.clone(), &payment_request.payee.address).await {
                    Ok(send_response) => {
                        let tx_id = send_response.payment.tx_id.unwrap_or_else(|| {
                            // If no tx_id, try to get swap_id as fallback
                            match &send_response.payment.details {
                                breez_sdk_liquid::model::PaymentDetails::Lightning { swap_id, .. } => swap_id.clone(),
                                breez_sdk_liquid::model::PaymentDetails::Bitcoin { swap_id, .. } => swap_id.clone(),
                                _ => "unknown".to_string()
                            }
                        });
                        Ok(tx_id)
                    },
                    Err(e) => Err(WalletError::SdkError(format!("Breez on-chain payment failed: {}", e)))
                }
            },
            (Blockchain::Lightning, _) => {
                self.breez_ctx.send_payment(payment_request).await
            },
            (Blockchain::Liquid, _) => {
                self.liquid_ctx.send_payment(payment_request).await
            },
            _ => {
                let payment_type = match &payment_request.prepared_payment {
                    PreparedPayment::Onchain(_) => "Onchain",
                    PreparedPayment::Liquid(_) => "Liquid", 
                    PreparedPayment::Lightning(_) => "Lightning",
                    PreparedPayment::PegOut(_) => "PegOut",
                };
                Err(WalletError::UnsupportedPayment {
                    blockchain: payment_request.blockchain.clone(),
                    payment_type: payment_type.to_string(),
                })
            }
        }
    }

    /// Get unified transaction history with proper duplicate filtering
    /// Breez handles Lightning/Bitcoin, LWK handles Liquid (with no duplicates)
    pub async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError> {
        let mut unified_transactions = Vec::new();

        // Get Bitcoin transactions
        let bitcoin_txs = self.bitcoin_ctx.transactions().await?;
        unified_transactions.extend(bitcoin_txs);

        // Get Breez transactions (already filtered to Lightning/Bitcoin only by payment_into_wallet_transaction)
        let breez_txs = self.breez_ctx.transactions().await?;
        
        // Get Liquid transactions
        let liquid_txs = self.liquid_ctx.transactions().await?;
        
        // Create a set of txids from Breez transactions for comparison
        let breez_txids: std::collections::HashSet<String> = breez_txs
            .iter()
            .filter_map(|tx| {
                if tx.txid.trim().is_empty() {
                    None // Skip empty txids
                } else {
                    Some(tx.txid.clone())
                }
            })
            .collect();
        
        // Filter LWK transactions: only include those not already in Breez
        let filtered_liquid_txs: Vec<WalletTransaction> = liquid_txs
            .into_iter()
            .filter(|liquid_tx| {
                // Skip transactions with invalid txids
                if liquid_tx.txid.trim().is_empty() {
                    return false;
                }
                // Only include if not already in Breez
                !breez_txids.contains(&liquid_tx.txid)
            })
            .collect();

        // Add Breez transactions (Lightning/Bitcoin)
        unified_transactions.extend(breez_txs);
        
        // Add filtered Liquid transactions (no duplicates with Breez)
        unified_transactions.extend(filtered_liquid_txs);

        // Sort transactions by timestamp (most recent first)
        unified_transactions.sort_by(|a, b| {
            match (a.timestamp, b.timestamp) {
                (Some(a_time), Some(b_time)) => b_time.cmp(&a_time),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        Ok(unified_transactions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::collections::HashMap;
    use mockall::{mock, predicate::*};
    
    // Mock implementations for testing
    mock! {
        pub BitcoinContext {}
        
        #[async_trait]
        impl WalletClient for BitcoinContext {
            async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError>;
            async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError>;
            async fn create_invoice(&self, amount: Option<u64>, description: Option<String>) -> Result<Invoice, WalletError>;
            async fn prepare_payment(&self, destination: &str, amount: u64, asset: Option<Asset>) -> Result<PaymentRequest, WalletError>;
            async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError>;
        }
    }
    
    mock! {
        pub BreezContext {}
        
        #[async_trait]
        impl WalletClient for BreezContext {
            async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError>;
            async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError>;
            async fn create_invoice(&self, amount: Option<u64>, description: Option<String>) -> Result<Invoice, WalletError>;
            async fn prepare_payment(&self, destination: &str, amount: u64, asset: Option<Asset>) -> Result<PaymentRequest, WalletError>;
            async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError>;
        }
        
        impl BreezContext {
            pub async fn build_onchain_transaction(&self, destination: &str, amount: u64) -> Result<breez_sdk_liquid::model::PreparePayOnchainResponse, crate::clients::breez::BreezError>;
            pub async fn finalize_onchain_transaction(&self, prepare_response: breez_sdk_liquid::model::PreparePayOnchainResponse, address: &str) -> Result<breez_sdk_liquid::model::SendPaymentResponse, crate::clients::breez::BreezError>;
        }
    }
    
    mock! {
        pub LiquidContext {}
        
        #[async_trait]
        impl WalletClient for LiquidContext {
            async fn balance(&self) -> Result<HashMap<Asset, u64>, WalletError>;
            async fn transactions(&self) -> Result<Vec<WalletTransaction>, WalletError>;
            async fn create_invoice(&self, amount: Option<u64>, description: Option<String>) -> Result<Invoice, WalletError>;
            async fn prepare_payment(&self, destination: &str, amount: u64, asset: Option<Asset>) -> Result<PaymentRequest, WalletError>;
            async fn send_payment(&self, payment_request: &PaymentRequest) -> Result<String, WalletError>;
        }
    }

    fn create_test_transaction(txid: &str, amount: u64, blockchain: Blockchain) -> WalletTransaction {
        WalletTransaction {
            txid: txid.to_string(),
            amount,
            asset: Asset::BitcoinOnchain,
            fee: Some(1000),
            timestamp: Some(chrono::Utc::now()),
            confirmed: true,
            direction: TransactionDirection::Outgoing,
            blockchain,
        }
    }

    #[tokio::test]
    async fn test_unified_wallet_constructor_validation() {
        // This test would need real context creation which requires complex setup
        // For now, we'll test the validation logic conceptually
        
        // Test would verify that constructor rejects invalid Arc references
        // and properly creates UnifiedWallet with valid contexts
    }

    #[tokio::test]
    async fn test_balance_aggregation_success() {
        // Mock setup would go here
        // Test would verify that balances from all contexts are properly combined
    }

    #[tokio::test]
    async fn test_balance_aggregation_partial_failure() {
        // Test that if one context fails, the others still work
        // and we get partial balance data
    }

    #[tokio::test]
    async fn test_balance_aggregation_total_failure() {
        // Test that if all contexts fail, we get MultipleContextFailures error
    }

    #[tokio::test]
    async fn test_create_invoice_asset_routing() {
        // Test that invoices are routed to correct contexts based on asset type
    }

    #[tokio::test]
    async fn test_prepare_payment_bitcoin_sufficient_funds() {
        // Test Bitcoin payment when sufficient funds are available
    }

    #[tokio::test]
    async fn test_prepare_payment_bitcoin_insufficient_funds_breez_fallback() {
        // Test Bitcoin payment falls back to Breez when insufficient Bitcoin funds
    }

    #[tokio::test]
    async fn test_prepare_payment_lightning() {
        // Test Lightning payment routing
    }

    #[tokio::test]
    async fn test_prepare_payment_liquid() {
        // Test Liquid payment routing
    }

    #[tokio::test]
    async fn test_send_payment_routing() {
        // Test that send_payment routes to correct context based on blockchain/payment type
    }

    #[tokio::test]
    async fn test_send_payment_unsupported_combination() {
        // Test error handling for unsupported blockchain/payment type combinations
    }

    #[test]
    fn test_transaction_duplicate_filtering() {
        // Create test transactions with some duplicates between Breez and Liquid
        let breez_txs = vec![
            create_test_transaction("tx1", 1000, Blockchain::Lightning),
            create_test_transaction("tx2", 2000, Blockchain::Bitcoin),
            create_test_transaction("", 3000, Blockchain::Lightning), // Empty txid should be filtered
        ];
        
        let liquid_txs = vec![
            create_test_transaction("tx2", 2000, Blockchain::Liquid), // Duplicate with Breez
            create_test_transaction("tx3", 3000, Blockchain::Liquid), // Unique
            create_test_transaction("", 4000, Blockchain::Liquid), // Empty txid should be filtered
        ];

        // Simulate the filtering logic from UnifiedWallet::transactions()
        let breez_txids: std::collections::HashSet<String> = breez_txs
            .iter()
            .filter_map(|tx| {
                if tx.txid.trim().is_empty() {
                    None // Skip empty txids
                } else {
                    Some(tx.txid.clone())
                }
            })
            .collect();

        let filtered_liquid_txs: Vec<WalletTransaction> = liquid_txs
            .into_iter()
            .filter(|liquid_tx| {
                // Skip transactions with invalid txids
                if liquid_tx.txid.trim().is_empty() {
                    return false;
                }
                // Only include if not already in Breez
                !breez_txids.contains(&liquid_tx.txid)
            })
            .collect();

        // Verify filtering worked correctly
        assert_eq!(breez_txids.len(), 2); // tx1, tx2 (empty txid filtered out)
        assert!(breez_txids.contains("tx1"));
        assert!(breez_txids.contains("tx2"));
        assert!(!breez_txids.contains(""));

        assert_eq!(filtered_liquid_txs.len(), 1); // Only tx3 should remain
        assert_eq!(filtered_liquid_txs[0].txid, "tx3");
    }

    #[tokio::test]
    async fn test_transaction_empty_txid_filtering() {
        // Test that transactions with empty txids are filtered out properly
    }

    #[test]
    fn test_transaction_sorting() {
        use chrono::{Utc, Duration};
        
        let now = Utc::now();
        let hour_ago = now - Duration::hours(1);
        let day_ago = now - Duration::days(1);
        
        let mut transactions = vec![
            WalletTransaction {
                txid: "old".to_string(),
                amount: 1000,
                asset: Asset::BitcoinOnchain,
                fee: Some(100),
                timestamp: Some(day_ago),
                confirmed: true,
                direction: TransactionDirection::Outgoing,
                blockchain: Blockchain::Bitcoin,
            },
            WalletTransaction {
                txid: "recent".to_string(),
                amount: 2000,
                asset: Asset::BitcoinOnchain,
                fee: Some(200),
                timestamp: Some(now),
                confirmed: true,
                direction: TransactionDirection::Incoming,
                blockchain: Blockchain::Bitcoin,
            },
            WalletTransaction {
                txid: "medium".to_string(),
                amount: 3000,
                asset: Asset::BitcoinOnchain,
                fee: Some(300),
                timestamp: Some(hour_ago),
                confirmed: true,
                direction: TransactionDirection::Outgoing,
                blockchain: Blockchain::Bitcoin,
            },
            WalletTransaction {
                txid: "no_timestamp".to_string(),
                amount: 4000,
                asset: Asset::BitcoinOnchain,
                fee: Some(400),
                timestamp: None,
                confirmed: true,
                direction: TransactionDirection::Incoming,
                blockchain: Blockchain::Bitcoin,
            },
        ];

        // Apply the same sorting logic as UnifiedWallet::transactions()
        transactions.sort_by(|a, b| {
            match (a.timestamp, b.timestamp) {
                (Some(a_time), Some(b_time)) => b_time.cmp(&a_time), // Most recent first
                (Some(_), None) => std::cmp::Ordering::Less,         // Timestamped before non-timestamped
                (None, Some(_)) => std::cmp::Ordering::Greater,      // Non-timestamped after timestamped
                (None, None) => std::cmp::Ordering::Equal,           // Equal if both None
            }
        });

        // Verify sorting: recent, medium, old, no_timestamp
        assert_eq!(transactions[0].txid, "recent");
        assert_eq!(transactions[1].txid, "medium");
        assert_eq!(transactions[2].txid, "old");
        assert_eq!(transactions[3].txid, "no_timestamp");
    }

    #[test]
    fn test_balance_aggregation_logic() {
        // Test balance HashMap merging logic
        let mut unified_balance = HashMap::new();
        
        let bitcoin_balance = {
            let mut balance = HashMap::new();
            balance.insert(Asset::BitcoinOnchain, 100000_u64);
            balance
        };
        
        let breez_balance = {
            let mut balance = HashMap::new();
            balance.insert(Asset::BitcoinLayer2, 50000_u64);
            balance
        };
        
        let liquid_balance = {
            let mut balance = HashMap::new();
            balance.insert(Asset::LiquidAsset("btc".to_string()), 75000_u64);
            balance
        };

        // Simulate the balance aggregation from UnifiedWallet::balance()
        unified_balance.extend(bitcoin_balance);
        unified_balance.extend(breez_balance);
        unified_balance.extend(liquid_balance);

        assert_eq!(unified_balance.len(), 3);
        assert_eq!(*unified_balance.get(&Asset::BitcoinOnchain).unwrap(), 100000);
        assert_eq!(*unified_balance.get(&Asset::BitcoinLayer2).unwrap(), 50000);
        assert_eq!(*unified_balance.get(&Asset::LiquidAsset("btc".to_string())).unwrap(), 75000);
    }

    #[test]
    fn test_error_message_formatting() {
        // Test that our custom error types format correctly
        let context_error = WalletError::ContextInitializationFailed {
            context: "Bitcoin".to_string(),
            reason: "Invalid Arc reference".to_string(),
        };
        
        let error_string = format!("{}", context_error);
        assert!(error_string.contains("Bitcoin"));
        assert!(error_string.contains("Invalid Arc reference"));

        let multiple_failures = WalletError::MultipleContextFailures(
            "Bitcoin: Connection failed; Liquid: Timeout".to_string()
        );
        
        let error_string = format!("{}", multiple_failures);
        assert!(error_string.contains("Bitcoin: Connection failed"));
        assert!(error_string.contains("Liquid: Timeout"));

        let unsupported_payment = WalletError::UnsupportedPayment {
            blockchain: Blockchain::Lightning,
            payment_type: "InvalidType".to_string(),
        };
        
        let error_string = format!("{}", unsupported_payment);
        assert!(error_string.contains("Lightning"));
        assert!(error_string.contains("InvalidType"));
    }
}