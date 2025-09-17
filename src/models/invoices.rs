use breez_sdk_liquid::model::{PrepareReceiveResponse, ReceivePaymentResponse};

#[derive(Debug)]
pub enum InvoiceType {
    Lightning(ReceivePaymentResponse),
    PegIn(ReceivePaymentResponse),
    Liquid,
    Onchain
}

#[derive(Debug)]
pub struct Invoice {
    pub address: String, 
    pub invoice_type: InvoiceType,
    pub amount: Option<u64>,
    pub description: Option<String>,
    pub fee: Option<u64>,
}