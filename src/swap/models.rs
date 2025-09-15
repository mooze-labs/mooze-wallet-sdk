use crate::models::Asset;

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
    pub expire_at: i64,
}

impl SwapRequestResponse {
    fn expired(&self) -> bool {
        chrono::Utc::now().timestamp() > self.expire_at
    }
    
    fn swap_rate(&self) -> u64 {
        self.send_amount / self.recv_amount
    }
}

pub struct SuccessfulSwap {
    txid: String,
    from: Asset,
    to: Asset,
    send_amount: u64,
    recv_amount: u64,
    fees: u64
}