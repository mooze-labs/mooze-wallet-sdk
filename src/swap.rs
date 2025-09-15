use std::sync::Arc;

use crate::clients::bitcoin::BitcoinCtx;
use crate::clients::{breez::BreezCtx, liquid::LwkCtx};
use crate::models::Asset;
use crate::swap::api::sideswap::SideswapClient;


mod api;
pub mod errors;
pub mod models;
pub mod traits;

pub use errors::*;
pub use models::*; 

const BOLTZ_SWAP_RATE: f64 = 0.1 / 100.0;
const ASSET_PRECISION: u64 = 10_u64.pow(8);

#[async_trait::async_trait]
pub trait SwapClient {
    async fn get_swap_rate(&self, from: Asset, to: Asset) -> f64;

    async fn prepare_swap(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError>;
    async fn confirm_swap(&self, swap_request: &SwapRequestResponse) -> Result<String, SwapError>;
}

pub struct Swapper {
    bitcoin_ctx: Arc<BitcoinCtx>,
    breez_ctx: Arc<BreezCtx>,
    liquid_ctx: Arc<LwkCtx>,
    sideswap_client: Box<SideswapClient>
}

impl Swapper {
    pub fn new(
        bitcoin_ctx: Arc<BitcoinCtx>,
        breez_ctx: Arc<BreezCtx>, 
        liquid_ctx: Arc<LwkCtx>,
        mainnet: bool
    ) -> Result<Swapper, SwapError> {
        let sideswap_client = Box::new(
            SideswapClient::new(mainnet).map_err(|e| SwapError::ConnectionError)?
        );

        Ok(Swapper { bitcoin_ctx, breez_ctx, liquid_ctx, sideswap_client })
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
            &change_addr?).await?;

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
            return self.peg_out().await;
        }

        todo!()
    }

    async fn peg_in(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError> {
        todo!()
    }

    async fn peg_out(&self) -> Result<SwapRequestResponse, SwapError> {
        todo!()
    }

    async fn generate_liquid_address(&self) -> Result<String, SwapError> {
        let wollet = self.liquid_ctx.wollet.read();
        let address = wollet.address(None)
            .map_err(|e| SwapError::ContextError("Failed to retrieve address".to_string()))?
            .address()
            .to_string();

        Ok(address)
    }

    async fn collect_swap_utxos(&self, asset: Asset, amount: u64) -> Result<Vec<lwk_wollet::WalletTxOut>, SwapError> {
        if let Asset::LiquidAsset(asset_id) = asset {
            let wollet = self.liquid_ctx.wollet.read();
            let mut accumulated = 0u64;
            let utxos = wollet.utxos()
                .map_err(|e| SwapError::ContextError(e.to_string()))?
                .into_iter()
                .filter(|utxo| utxo.unblinded.asset.to_string() == asset_id)
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
}

fn get_asset_id(asset: &Asset) -> Result<String, SwapError> {
    match asset {
        Asset::BitcoinLayer2 => Ok(lwk_wollet::elements::AssetId::LIQUID_BTC.to_string()),
        Asset::LiquidAsset(asset_id) => Ok(asset_id.clone()),
        _ => Err(SwapError::InvalidAsset)
    }
}