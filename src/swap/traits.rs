use super::errors::SwapError;
use super::models::*;

use crate::models::Asset;

#[async_trait::async_trait]
pub trait SwapClient {
    async fn get_swap_rate(&self, from: Asset, to: Asset) -> f64;

    async fn prepare_swap(&self, swap_request: &SwapRequest) -> Result<SwapRequestResponse, SwapError>;
    async fn confirm_swap(&self, swap_request: &SwapRequestResponse) -> Result<String, SwapError>;
}