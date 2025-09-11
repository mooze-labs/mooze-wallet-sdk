use crate::models::Asset;

pub enum SwapDirection {
    Buy,
    Sell
}

pub struct SwapRequest {
    from: Asset,
    to: Asset,
    swap_direction: SwapDirection,
    amount: u64,
}

pub struct SwapRequestResponse {
    from: Asset,
    to: Asset,
    send_amount: u64,
    recv_amount: u64,
    fees: u64,
    expire_at: i64,
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