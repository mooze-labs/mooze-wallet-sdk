use breez_sdk_liquid::model::ReceivePaymentResponse;

pub enum InvoiceType {
    Lightning(ReceivePaymentResponse),
    PegIn(ReceivePaymentResponse),
    Liquid,
    Onchain
}

pub struct Invoice {
    pub address: String, 
    pub invoice_type: InvoiceType,
    pub amount: Option<u64>,
    pub description: Option<String>,
    pub fee: Option<u64>,
}