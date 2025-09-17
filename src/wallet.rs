use crate::{clients::{bitcoin::BitcoinCtx, breez::BreezCtx, liquid::LwkCtx}, models::WalletConfig};

pub mod traits;

const STOP_GAP: usize = 20;
const BATCH_SIZE: usize = 5;

const BREEZ_API_KEY: &str = "MIIBajCCARygAwIBAgIHPgbGnsVq8TAFBgMrZXAwEDEOMAwGA1UEAxMFQnJlZXowHhcNMjUwNDI5MDEyNDI5WhcNMzUwNDI3MDEyNDI5WjArMRMwEQYDVQQKEwpNb296ZSBMYWJzMRQwEgYDVQQDEwtMdWNjYSBHb2RveTAqMAUGAytlcAMhANCD9cvfIDwcoiDKKYdT9BunHLS2/OuKzV8NS0SzqV13o3oweDAOBgNVHQ8BAf8EBAMCBaAwDAYDVR0TAQH/BAIwADAdBgNVHQ4EFgQU2jmj7l5rSw0yVb/vlWAYkK/YBwkwHwYDVR0jBBgwFoAU3qrWklbzjed0khb8TLYgsmsomGswGAYDVR0RBBEwD4ENZGV2QG1vb3plLmFwcDAFBgMrZXADQQAx9hoGj97ubdjFT/C7KqEZOOSVV2C8HHIw4D6//NG9mEJPB1Mc9HTvWmEFaIKhz1vdH6z5zQDyw9RJV4Ej7tEL";
const BITCOIN_ELECTRUM_URL: &str = "electrum.blockstream.info:700";
const LIQUID_ELECTRUM_URL: &str = "electrum.blockstream.info:995";
const WORKING_DIR: &str = "";

pub(crate) struct WalletCtx {
    bitcoin_client: Option<Box<BitcoinCtx>>,
    breez_client: Option<Box<BreezCtx>>,
    liquid_client: Option<Box<LwkCtx>>,
    pub config: WalletConfig
}

impl WalletCtx {
    pub async fn new(config: WalletConfig, mnemonic: &str) -> Result<Self, anyhow::Error> {
        rustls::crypto::ring::default_provider().install_default().expect("Failed to install rustls crypto provider");

        let mnemonic = mnemonic.to_string();
        let config_clone1 = config.clone();
        let config_clone2 = config.clone();
        let config_clone3 = config.clone();
        
        let (bitcoin_result, breez_result, liquid_result) = tokio::join!(
            tokio::spawn({
                let mnemonic = mnemonic.clone();
                let config = config_clone1;
                async move { 
                    let ctx = BitcoinCtx::new(&mnemonic, &config).map_err(anyhow::Error::from)?;
                    ctx.full_scan().map_err(anyhow::Error::from)?;
                    Ok::<BitcoinCtx, anyhow::Error>(ctx)
                }
            }),
            tokio::spawn({
                let mnemonic = mnemonic.clone();
                let config = config_clone2;
                async move { BreezCtx::new(&mnemonic, &config).await }
            }),
            tokio::spawn({
                let mnemonic = mnemonic.clone();
                let config = config_clone3;
                async move { LwkCtx::new(&mnemonic, &config) }
            })
        );

        let bitcoin_client = match bitcoin_result {
            Ok(Ok(client)) => Some(Box::new(client)),
            Ok(Err(e)) => {
                eprintln!("Failed to initialize Bitcoin client: {}", e);
                None
            }
            Err(e) => {
                eprintln!("Bitcoin client task panicked: {}", e);
                None
            }
        };

        let breez_client = match breez_result {
            Ok(Ok(client)) => Some(Box::new(client)),
            Ok(Err(e)) => {
                eprintln!("Failed to initialize Breez client: {}", e);
                None
            }
            Err(e) => {
                eprintln!("Breez client task panicked: {}", e);
                None
            }
        };

        let liquid_client = match liquid_result {
            Ok(Ok(client)) => Some(Box::new(client)),
            Ok(Err(e)) => {
                eprintln!("Failed to initialize Liquid client: {}", e);
                None
            }
            Err(e) => {
                eprintln!("Liquid client task panicked: {}", e);
                None
            }
        };

        Ok(WalletCtx {
            bitcoin_client,
            breez_client,
            liquid_client,
            config
        })
    }

    pub fn is_bitcoin_available(&self) -> bool {
        self.bitcoin_client.is_some()
    }

    pub fn is_breez_available(&self) -> bool {
        self.breez_client.is_some()
    }

    pub fn is_liquid_available(&self) -> bool {
        self.liquid_client.is_some()
    }

    pub fn bitcoin_client(&self) -> Option<&BitcoinCtx> {
        self.bitcoin_client.as_ref().map(|client| client.as_ref())
    }

    pub fn breez_client(&self) -> Option<&BreezCtx> {
        self.breez_client.as_ref().map(|client| client.as_ref())
    }

    pub fn liquid_client(&self) -> Option<&LwkCtx> {
        self.liquid_client.as_ref().map(|client| client.as_ref())
    }

    pub fn available_networks(&self) -> Vec<&str> {
        let mut networks = Vec::new();
        if self.is_bitcoin_available() {
            networks.push("Bitcoin");
        }
        if self.is_breez_available() {
            networks.push("Lightning");
        }
        if self.is_liquid_available() {
            networks.push("Liquid");
        }
        networks
    }
}