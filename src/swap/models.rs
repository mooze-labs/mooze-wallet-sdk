use breez_sdk_liquid::model::{PreparePayOnchainResponse, PrepareReceiveResponse, ReceivePaymentResponse};

use crate::{models::Asset, swap::api::sideswap::{PegOrder, QuoteStatus}};

pub(crate) enum BreezTransfer {
    PegOut(PreparePayOnchainResponse),
    PegIn(ReceivePaymentResponse)
}

pub(crate) enum SideswapTransfer {
    Swap(QuoteStatus),
    PegIn(PegOrder),
    PegOut(PegOrder)
}

pub(crate) enum SwapInner {
    BreezTransfer(BreezTransfer),
    SideswapTransfer(SideswapTransfer)
}

pub enum SwapDirection {
    Buy,
    Sell
}

pub struct SwapRequest {
    pub from: Asset,
    pub to: Asset,
    pub swap_direction: SwapDirection,
    pub amount: u64,
}

pub struct SwapRequestResponse {
    pub from: Asset,
    pub to: Asset,
    pub send_amount: u64,
    pub recv_amount: u64,
    pub fees: u64,
    pub expire_at: u64,
    pub(crate) inner: SwapInner
}

impl SwapRequestResponse {
    fn swap_rate(&self) -> u64 {
        self.send_amount / self.recv_amount
    }
}

pub struct PegOperation {
    pub peg_in: bool,
    pub order_id: String,
    pub lockup_txid: String,
    pub lockup_address: String,
}

pub struct SwapOperation {
    pub txid: String,
    pub from: Asset,
    pub to: Asset,
    pub send_amount: u64,
    pub recv_amount: u64,
    pub fees: u64
}

pub enum Swap {
    Peg(PegOperation),
    Swap(SwapOperation)
}