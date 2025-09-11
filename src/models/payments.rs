use bdk_wallet::bitcoin::Psbt;
use lwk_wollet::elements::pset::PartiallySignedTransaction;
use breez_sdk_liquid::model::{PreparePayOnchainResponse, PrepareSendResponse};

use super::*;

#[derive(Clone, Debug)]
pub enum PreparedPayment {
    Onchain(Psbt),
    Liquid(PartiallySignedTransaction),
    Lightning(PrepareSendResponse),
    PegOut(PreparePayOnchainResponse),
}

#[derive(Debug, Clone)]
pub struct Payee {
    pub address: String,
    pub asset: Asset,
    pub satoshi: u64,
}

#[derive(Debug, Clone)]
pub struct PaymentRequest {
    pub fees: u64,
    pub blockchain: Blockchain,
    pub payee: Payee,
    pub prepared_payment: PreparedPayment 
}

impl PaymentRequest { 
    pub fn new(
        address: &str,
        satoshi: u64,
        asset: Asset,
        fees: u64,
        blockchain: Blockchain,
        prepared_payment: PreparedPayment
    ) -> PaymentRequest {
        PaymentRequest {
            fees,
            blockchain,
            payee: Payee { address: address.to_string(), asset, satoshi },
            prepared_payment
        }
    }
}