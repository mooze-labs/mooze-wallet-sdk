use std::sync::Arc;

use crate::clients::{breez::BreezCtx, liquid::LwkCtx};
use crate::models::{swap::*, Asset};

pub mod errors;
pub mod models;
pub(crate) mod services;
pub mod traits;

struct Swapper {
    breez_ctx: Arc<BreezCtx>,
    liquid_ctx: Arc<LwkCtx>
}

impl Swapper {
    pub fn new(breez_ctx: Arc<BreezCtx>, liquid_ctx: Arc<LwkCtx>) -> Swapper {
        Swapper { breez_ctx, liquid_ctx }
    }

    async fn fetch_swap_rate(&self, send_asset: Asset, receive_asset: Asset) -> f64 {
        todo!()
    }
}