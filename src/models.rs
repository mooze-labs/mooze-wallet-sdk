pub mod invoices;
pub mod payments;
pub mod swap;

const STOP_GAP: usize = 20;
const BATCH_SIZE: usize = 5;

const BREEZ_API_KEY: &str = "MIIBajCCARygAwIBAgIHPgbGnsVq8TAFBgMrZXAwEDEOMAwGA1UEAxMFQnJlZXowHhcNMjUwNDI5MDEyNDI5WhcNMzUwNDI3MDEyNDI5WjArMRMwEQYDVQQKEwpNb296ZSBMYWJzMRQwEgYDVQQDEwtMdWNjYSBHb2RveTAqMAUGAytlcAMhANCD9cvfIDwcoiDKKYdT9BunHLS2/OuKzV8NS0SzqV13o3oweDAOBgNVHQ8BAf8EBAMCBaAwDAYDVR0TAQH/BAIwADAdBgNVHQ4EFgQU2jmj7l5rSw0yVb/vlWAYkK/YBwkwHwYDVR0jBBgwFoAU3qrWklbzjed0khb8TLYgsmsomGswGAYDVR0RBBEwD4ENZGV2QG1vb3plLmFwcDAFBgMrZXADQQAx9hoGj97ubdjFT/C7KqEZOOSVV2C8HHIw4D6//NG9mEJPB1Mc9HTvWmEFaIKhz1vdH6z5zQDyw9RJV4Ej7tEL";
const BITCOIN_ELECTRUM_URL: &str = "ssl://mempool.space:50002";
const LIQUID_ELECTRUM_URL: &str = "ssl://electrum.blockstream.info:50002";
const WORKING_DIR: &str = "";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Asset {
    BitcoinOnchain,
    BitcoinLayer2,
    LiquidAsset(String)
}

pub struct AssetBalance {
    pub asset_id: String,
    pub amount: u64
}

pub enum Balance {
    Onchain(u64),
    Liquid(u64),
    Asset(AssetBalance)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Blockchain {
    Bitcoin,
    Liquid,
    Lightning // technically not a blockchain but ok
}

#[derive(Clone, Debug)]
pub enum Network {
    Bitcoin,
    Liquid,
    Lightning
}

#[derive(Clone)]
pub enum NetworkType {
    Mainnet,
    Testnet,
    Regtest
}

#[derive(Clone)]
pub enum TransactionDirection {
    Incoming,
    Outgoing,
    Swap,
    PegIn,
    PegOut
}

#[derive(Clone)]
pub struct WalletTransaction {
    pub txid: String,
    pub amount: u64,
    pub asset: Asset,
    pub fee: Option<u64>,
    pub timestamp: Option<u64>,
    pub confirmed: bool,
    pub direction: TransactionDirection,
    pub blockchain: Blockchain
}

#[derive(Clone)]
pub struct WalletConfig {
    pub bitcoin_electrum_url: String,
    pub liquid_electrum_url: String,
    pub liquid_policy_asset: String,
    pub network_type: NetworkType,
    pub batch_size: usize,
    pub stop_gap: usize,
    pub working_dir: String,
}

impl Default for WalletConfig {
    fn default() -> WalletConfig {
        WalletConfig {
            bitcoin_electrum_url: BITCOIN_ELECTRUM_URL.to_string(),
            liquid_electrum_url: LIQUID_ELECTRUM_URL.to_string(),
            liquid_policy_asset: lwk_wollet::elements::issuance::AssetId::LIQUID_BTC.to_string(),
            network_type: NetworkType::Mainnet,
            batch_size: BATCH_SIZE,
            stop_gap: STOP_GAP,
            working_dir: WORKING_DIR.to_string()
        }
    }
}