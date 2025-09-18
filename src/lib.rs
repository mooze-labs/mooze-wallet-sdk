mod clients;
mod errors;
mod infra;
mod models;
mod swap;
mod traits;
mod wallet;
mod unified_wallet;

pub use unified_wallet::UnifiedWallet;
pub use models::{Asset, WalletConfig, WalletTransaction, NetworkType};
pub use models::invoices::Invoice;
pub use models::payments::PaymentRequest;
pub use errors::WalletError;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
