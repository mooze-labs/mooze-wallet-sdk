use std::str::FromStr;
use std::sync::Arc;

use crate::clients::bitcoin::BitcoinCtx;
use crate::clients::WalletClient;
use crate::clients::{breez::BreezCtx, liquid::LwkCtx};
use crate::models::{Asset, NetworkType};
use crate::swap::api::sideswap::{PegOrder, QuoteStatus, SideswapClient};
use crate::wallet::WalletCtx;


mod api;
pub mod errors;
pub mod models;
pub mod traits;

use breez_sdk_liquid::lightning::blinded_path::payment;
use breez_sdk_liquid::model::{PayAmount, PayOnchainRequest, Payment, PaymentDetails, PaymentMethod, PreparePayOnchainRequest, PreparePayOnchainResponse, ReceivePaymentRequest, ReceivePaymentResponse};
pub use errors::*;
use lwk_wollet::elements::pset::PartiallySignedTransaction;
pub use models::*; 

const BOLTZ_SWAP_RATE: f64 = 0.1 / 100.0;
const ASSET_PRECISION: u64 = 10_u64.pow(8);

const MIN_PEG_OUT_AMOUNT: u64 = 25000;
const MIN_PEG_IN_AMOUNT: u64 = 10000;

const SIDESWAP_PEG_RESERVATION_MINUTES: i64 = 60 * 6;

#[async_trait::async_trait]
pub trait SwapClient {
    async fn get_swap_rate(&self, from: Asset, to: Asset) -> f64;

    async fn prepare_swap(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError>;
    async fn confirm_swap(&self, swap_request: &SwapRequestResponse) -> Result<String, SwapError>;
}

pub struct Swapper {
    wallet_ctx: Arc<WalletCtx>,
    sideswap_client: Box<SideswapClient>
}

impl Swapper {
    pub fn new(
        wallet_ctx: Arc<WalletCtx>,
    ) -> Result<Swapper, SwapError> {
        let sideswap_client = Box::new(
            SideswapClient::new((wallet_ctx.config.network_type == NetworkType::Mainnet)).map_err(|e| SwapError::ConnectionError)?
        );

        Ok(Swapper { wallet_ctx, sideswap_client })
    }

    pub async fn fetch_swap_rate(&self, send_asset: Asset, receive_asset: Asset) -> Result<f64, SwapError> {
        if (send_asset == Asset::BitcoinOnchain) || (receive_asset == Asset::BitcoinOnchain) {
            return Ok(BOLTZ_SWAP_RATE);
        }

        let (recv_addr, change_addr) = tokio::join!(
            self.generate_liquid_address(),
            self.generate_liquid_address()
        );

        let rate = self.sideswap_client.fetch_current_rate(
            &get_asset_id(&send_asset)?, 
            &get_asset_id(&receive_asset)?,
            &recv_addr?,
            &change_addr?)
            .await?;

        Ok(rate)
    }

    pub async fn request_swap(
        &self, 
        swap_request: &SwapRequest
    ) -> Result<SwapRequestResponse, SwapError> {
        if swap_request.from.eq(&swap_request.to) {
            return Err(SwapError::InvalidMarket)
        }

        if (swap_request.from == Asset::BitcoinOnchain) {
            return self.peg_in(swap_request).await;
        }

        if (swap_request.to == Asset::BitcoinOnchain) {
            return self.peg_out(swap_request).await;
        }

        self.swap_liquid_assets(swap_request).await
    }

    pub async fn confirm_swap(&self, swap_request_response: &SwapRequestResponse) -> Result<Swap, SwapError> {
        match swap_request_response.inner {
            SwapInner::SideswapTransfer(_) => self.confirm_sideswap_transfer(swap_request_response).await,
            _ => Err(SwapError::ContextError("Breez operations deactivated".to_string()))
        }
    }

    async fn peg_in(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        if swap_request.from != Asset::BitcoinOnchain { return Err(SwapError::InvalidMarket) };
        if swap_request.amount < MIN_PEG_IN_AMOUNT { return Err(SwapError::AmountBelowMinimum(MIN_PEG_IN_AMOUNT)) };

        // For now, breez will be deactivated
        return self.sideswap_peg_in(swap_request).await
    }

    async fn peg_out(&self, swap_request: &SwapRequest)-> Result<SwapRequestResponse, SwapError> {
        if swap_request.from != Asset::BitcoinLayer2 { return Err(SwapError::InvalidMarket) };
        if swap_request.amount < MIN_PEG_OUT_AMOUNT { return Err(SwapError::AmountBelowMinimum(MIN_PEG_OUT_AMOUNT)) };

        // For now, breez will be deactivated
        return self.sideswap_peg_out(swap_request).await
    }

    async fn breez_peg_in(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        let breez_ctx = self.get_breez_context().await?;

        let limits = breez_ctx.sdk.fetch_onchain_limits()
            .await
            .map_err(|e| SwapError::ContextError(e.to_string()))?;

        if swap_request.amount < limits.receive.min_sat {
            return Err(SwapError::AmountBelowMinimum(limits.receive.min_sat));
        }

        if swap_request.amount > limits.receive.max_sat {
            return Err(SwapError::AmountAboveMax(limits.receive.max_sat));
        }

        let prepare_response = breez_ctx.sdk.prepare_receive_payment(&breez_sdk_liquid::model::PrepareReceiveRequest { 
            payment_method: PaymentMethod::BitcoinAddress, amount: Some(breez_sdk_liquid::model::ReceiveAmount::Bitcoin { payer_amount_sat: swap_request.amount })
        })
        .await
        .map_err(|e| SwapError::ContextError(e.to_string()))?;
    
        let payment = breez_ctx.sdk.receive_payment(&ReceivePaymentRequest {
            prepare_response: prepare_response.clone(),
            description: None,
            use_description_hash: None,
            payer_note: None
        })
        .await
        .map_err(|e| SwapError::ContextError(e.to_string()))?;

        let recv_amount = swap_request.amount - ((swap_request.amount as f64 * &prepare_response.swapper_feerate.unwrap_or(BOLTZ_SWAP_RATE)).ceil() as u64);

        let swap_response = SwapRequestResponse {
            from: swap_request.from.clone(),
            to: swap_request.to.clone(),
            send_amount: swap_request.amount,
            recv_amount,
            fees: prepare_response.fees_sat,
            expire_at: (chrono::Utc::now() + chrono::Duration::minutes(15)).timestamp() as u64,
            inner: SwapInner::BreezTransfer(BreezTransfer::PegIn(payment))
        };

        Ok(swap_response)
    }

    async fn breez_peg_out(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        let breez_ctx = self.get_breez_context().await?;

        let limits = breez_ctx.sdk.fetch_onchain_limits()
            .await
            .map_err(|e| SwapError::ContextError(e.to_string()))?;

        if swap_request.amount < limits.send.min_sat {
            return Err(SwapError::AmountBelowMinimum(limits.receive.min_sat));
        }

        if swap_request.amount > limits.send.max_sat {
            return Err(SwapError::AmountAboveMax(limits.receive.max_sat));
        }

        let prepare_res = breez_ctx.sdk
            .prepare_pay_onchain(&PreparePayOnchainRequest {
                amount: PayAmount::Bitcoin { receiver_amount_sat: swap_request.amount },
                fee_rate_sat_per_vbyte: None
            })
            .await
            .map_err(|e| SwapError::ContextError(e.to_string()))?;

        let swap_request = SwapRequestResponse {
            from: swap_request.from.clone(),
            to: swap_request.to.clone(),
            send_amount: (prepare_res.receiver_amount_sat + prepare_res.total_fees_sat),
            recv_amount: prepare_res.receiver_amount_sat,
            fees: prepare_res.total_fees_sat,
            inner: SwapInner::BreezTransfer(BreezTransfer::PegOut(prepare_res)),
            expire_at: (chrono::Utc::now() + chrono::Duration::minutes(15)).timestamp() as u64,
        };

        Ok(swap_request)
    }

    async fn sideswap_peg_in(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        let bitcoin_ctx = self.get_bitcoin_context().await?;
        let wallet = bitcoin_ctx.wallet.read();
        
        if wallet.balance().trusted_spendable().to_sat() < swap_request.amount {
            return Err(SwapError::InsufficientFunds);
        }

        let peg_in_address = self.generate_liquid_address().await?;
        let peg_in = self.sideswap_client.peg(true, &peg_in_address).await?;

        let fees = (swap_request.amount + 999) / 1000;

        let swap_request = SwapRequestResponse {
            from: swap_request.from.clone(),
            to: swap_request.to.clone(),
            send_amount: swap_request.amount,
            recv_amount: swap_request.amount - fees,
            fees,
            inner: SwapInner::SideswapTransfer(SideswapTransfer::PegIn(peg_in)),
            expire_at: (chrono::Utc::now() + chrono::Duration::minutes(SIDESWAP_PEG_RESERVATION_MINUTES)).timestamp() as u64,
        };

        Ok(swap_request)
    }

    async fn sideswap_peg_out(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        let bitcoin_ctx = self.get_bitcoin_context().await?;
        let mut wallet = bitcoin_ctx.wallet.write();

        let lwk_ctx = self.get_liquid_context().await?;
        let balances = lwk_ctx.wollet.read()
            .balance()
            .map_err(|e| SwapError::ContextError(e.to_string()))?;

        let lbtc_balance = balances
            .get(&lwk_wollet::elements::AssetId::LIQUID_BTC)
            .ok_or_else(|| SwapError::InsufficientFunds)?
            .to_owned();

        if lbtc_balance < swap_request.amount {
            return Err(SwapError::InsufficientFunds);
        }
        
        let peg_out_address = wallet.next_unused_address(bdk_wallet::KeychainKind::External).address.to_string();
        let peg_out = self.sideswap_client.peg(false, &peg_out_address).await?;

        let fees = (swap_request.amount + 999) / 1000;

        let swap_request = SwapRequestResponse {
            from: swap_request.from.clone(),
            to: swap_request.to.clone(),
            send_amount: swap_request.amount,
            recv_amount: swap_request.amount - fees,
            fees,
            inner: SwapInner::SideswapTransfer(SideswapTransfer::PegOut(peg_out)),
            expire_at: (chrono::Utc::now() + chrono::Duration::minutes(SIDESWAP_PEG_RESERVATION_MINUTES)).timestamp() as u64,
        };

        Ok(swap_request)
    }

    async fn swap_liquid_assets(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        let (recv_addr, change_addr) = tokio::join!(
            self.generate_liquid_address(),
            self.generate_liquid_address()
        );
        let utxos = self.collect_swap_utxos(&swap_request.from, swap_request.amount).await?;

        let quote = self.sideswap_client.request_swap(
            &swap_request.from.asset_id().ok_or_else(|| SwapError::InvalidAsset)?, 
            &swap_request.to.asset_id().ok_or_else(|| SwapError::InvalidAsset)?, 
            swap_request.amount, 
            &recv_addr?, 
            &change_addr?, 
            utxos
        ).await?;

        if let QuoteStatus::Success { quote_id, base_amount, quote_amount, server_fee, fixed_fee, ttl } = quote {
            // Workaround to get if base asset or quote asset based on quotestatus
            let base_amount_diff = swap_request.amount.abs_diff(base_amount);
            let quote_amount_diff = swap_request.amount.abs_diff(quote_amount);

            let response = SwapRequestResponse {
                from: swap_request.from.clone(),
                to: swap_request.to.clone(),
                send_amount: swap_request.amount,
                recv_amount: if base_amount_diff < quote_amount_diff { quote_amount } else { base_amount },
                fees: server_fee + fixed_fee,
                expire_at: ttl,
                inner: SwapInner::SideswapTransfer(SideswapTransfer::Swap(quote))
            };

            return Ok(response);
        }

        Err(SwapError::DealerUnavailable)

    }

    async fn confirm_breez_transfer(&self, swap_request_response: &SwapRequestResponse) -> Result<String, SwapError> {
        match &swap_request_response.inner {
            SwapInner::BreezTransfer(transfer) => match transfer {
                BreezTransfer::PegIn(peg_in) => {
                    let peg_in_tx = self.confirm_breez_peg_in(&peg_in.destination, swap_request_response.send_amount).await?;
                    Ok(peg_in_tx)
                },
                BreezTransfer::PegOut(peg_out) => self.confirm_breez_peg_out(&peg_out).await,
            },
            _ => Err(SwapError::ArgumentError("Breez context may only receive breez transfers.".to_string()))
        }
    }

    async fn confirm_breez_peg_in(&self, destination: &str, amount: u64) -> Result<String, SwapError> {
        let btc_ctx = self.get_bitcoin_context().await?;
        let payment_request = btc_ctx.prepare_payment(
            destination, 
            amount, 
            None
        ).await?;
        let confirm_payment = btc_ctx.send_payment(&payment_request).await?;

        Ok(confirm_payment)
    }

    async fn confirm_breez_peg_out(&self, prepare_on_chain_response: &PreparePayOnchainResponse) -> Result<String, SwapError> {
        let (breez_ctx, bitcoin_ctx) = tokio::join!(
            self.get_breez_context(),
            self.get_bitcoin_context()
        );

        let destination_addr = bitcoin_ctx?.wallet.write()
            .next_unused_address(bdk_wallet::KeychainKind::External)
            .address
            .to_string();

        let payment = breez_ctx?.sdk
            .pay_onchain(&PayOnchainRequest { 
                address: destination_addr, 
                prepare_response: prepare_on_chain_response.clone() 
            }
        ).await
        .map_err(|e| SwapError::ContextError(e.to_string()))?;

        if let PaymentDetails::Bitcoin { swap_id, .. } = payment.payment.details {
            return Ok(swap_id);
        } else {
            return Err(SwapError::ArgumentError("Payment details are not bitcoin.".to_string()))
        }
    }

    async fn confirm_sideswap_transfer(&self, swap_request_response: &SwapRequestResponse) -> Result<Swap, SwapError> {
        match &swap_request_response.inner {
            SwapInner::SideswapTransfer(op) => match op {
                SideswapTransfer::Swap(swap) => {
                    let txid = self.confirm_asset_swap(&swap).await?;
                    let swap_op = SwapOperation {
                        txid,
                        from: swap_request_response.from.clone(),
                        to: swap_request_response.to.clone(),
                        send_amount: swap_request_response.send_amount,
                        recv_amount: swap_request_response.recv_amount,
                        fees: swap_request_response.fees
                    };

                    Ok(Swap::Swap(swap_op))
                },
                SideswapTransfer::PegIn(peg_in) => {
                    let txid = self.confirm_sideswap_pegin(&peg_in, swap_request_response.send_amount).await?;
                    let peg_operation = PegOperation {
                        peg_in: true,
                        order_id: peg_in.order_id.clone(),
                        lockup_txid: txid,
                        lockup_address: peg_in.peg_addr.clone(),
                    };

                    Ok(Swap::Peg(peg_operation))
                },
                SideswapTransfer::PegOut(peg_out) => {
                    let txid = self.confirm_sideswap_pegout(&peg_out, swap_request_response.send_amount).await?;
                    let peg_operation = PegOperation {
                        peg_in: false,
                        order_id: peg_out.order_id.clone(),
                        lockup_txid: txid,
                        lockup_address: peg_out.peg_addr.clone(),
                    };

                    Ok(Swap::Peg(peg_operation))
                }
            },
            _ => Err(SwapError::ArgumentError("Sideswap operations may only receive sideswap enums".to_string()))
        }
    }

    async fn confirm_asset_swap(&self, quote_status: &QuoteStatus) -> Result<String, SwapError> {
        if let QuoteStatus::Success { quote_id, ..} = quote_status {
            let pset = self.sideswap_client.retrieve_quote_pset(*quote_id).await?;
            let signed_pset = self.sign_liquid_swap_pset(&pset).await?;
            let txid = self.sideswap_client.sign_quote(*quote_id, signed_pset).await?;

            return Ok(txid);
        }

        Err(SwapError::InvalidQuote)
    }

    async fn confirm_sideswap_pegin(&self, peg_order: &PegOrder, amount: u64) -> Result<String, SwapError> {
        let btc_ctx = self.get_bitcoin_context().await?;
        let payment_request = btc_ctx.prepare_payment(
            &peg_order.peg_addr, 
            amount, 
            None
        ).await?;
        let confirm_payment = btc_ctx.send_payment(&payment_request).await?;

        Ok(confirm_payment)
    }

    async fn confirm_sideswap_pegout(&self, peg_order: &PegOrder, amount: u64) -> Result<String, SwapError> {
        let lwk_ctx = self.get_liquid_context().await?;
        let payment_request = lwk_ctx.prepare_payment(
            &peg_order.peg_addr, 
            amount, 
            Some(Asset::BitcoinLayer2)
        ).await?;
        let confirm_payment = lwk_ctx.send_payment(&payment_request).await?;

        Ok(confirm_payment)
    }

    async fn generate_liquid_address(&self) -> Result<String, SwapError> {
        let lwk_ctx = self.get_liquid_context().await?;
        let wollet = lwk_ctx.wollet.read();
        let address = wollet.address(None)
            .map_err(|e| SwapError::ContextError("Failed to retrieve address".to_string()))?
            .address()
            .to_string();

        Ok(address)
    }

    async fn collect_swap_utxos(&self, asset: &Asset, amount: u64) -> Result<Vec<lwk_wollet::WalletTxOut>, SwapError> {
        let lwk_ctx = self.get_liquid_context().await?;
        if let Asset::LiquidAsset(asset_id) = asset {
            let wollet = lwk_ctx.wollet.read();
            let mut accumulated = 0u64;
            let utxos = wollet.utxos()
                .map_err(|e| SwapError::ContextError(e.to_string()))?
                .into_iter()
                .filter(|utxo| utxo.unblinded.asset.to_string() == *asset_id)
                .scan(false, move |done, utxo| {
                    if *done {
                        return None;
                    }

                    accumulated += utxo.unblinded.value;
                    if accumulated >= amount {
                        *done = true;
                    }
                    Some(utxo)
                })
                .collect::<Vec<lwk_wollet::WalletTxOut>>();

            if utxos.iter().map(|utxo| utxo.unblinded.value).sum::<u64>() < amount {
                return Err(SwapError::InsufficientFunds);
            }

            return Ok(utxos);
        }

        Err(SwapError::InvalidMarket)
    }

    async fn sign_liquid_swap_pset(&self, pset: &str) -> Result<String, SwapError> {
        let lwk_ctx = self.get_liquid_context().await?;
        let pset = PartiallySignedTransaction::from_str(pset)
            .map_err(|e| SwapError::ContextError("Badly formatted PSET".to_string()))?;

        let signed_pset = lwk_ctx.sign_with_extra_details(&pset).map_err(|e| SwapError::ContextError(e.to_string()))?;
        Ok(signed_pset.to_string())
    }

    async fn get_bitcoin_context(&self) -> Result<&BitcoinCtx, SwapError> {
        let btc_ctx = self.wallet_ctx
            .bitcoin_client()
            .ok_or_else(|| SwapError::ContextError("Bitcoin context not available.".to_string()))?;

        Ok(btc_ctx)
    }

    async fn get_breez_context(&self) -> Result<&BreezCtx, SwapError> {
        let breez_ctx = self.wallet_ctx
            .breez_client()
            .ok_or_else(|| SwapError::ContextError("Breez context not available".to_string()))?;

        Ok(breez_ctx)
    }

    async fn get_liquid_context(&self) -> Result<&LwkCtx, SwapError> {
        let lwk_ctx = self.wallet_ctx
            .liquid_client()
            .ok_or_else(|| SwapError::ContextError("Liquid context not available".to_string()))?;

        Ok(lwk_ctx)
    }
}

fn get_asset_id(asset: &Asset) -> Result<String, SwapError> {
    match asset {
        Asset::BitcoinLayer2 => Ok(lwk_wollet::elements::AssetId::LIQUID_BTC.to_string()),
        Asset::LiquidAsset(asset_id) => Ok(asset_id.clone()),
        _ => Err(SwapError::InvalidAsset)
    }
}