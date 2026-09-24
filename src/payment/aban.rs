use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct AbanClient {
    http: Client,
    api_token: String,
    api_base_url: String,
}

#[derive(Serialize)]
struct CreateInvoiceRequest<'a> {
    amount_rial: i64,
    order_id: &'a str,
    callback_url: &'a str,
    description: &'a str,
    metadata: serde_json::Value,
    expiry_minutes: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AbanInvoice {
    pub invoice_id: String,
    pub status: String,
    pub amount_rial: i64,
    pub payable_rial: i64,
    pub payment_url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AbanVerification {
    pub verified: bool,
    pub invoice_id: String,
    pub order_id: String,
    pub amount_rial: i64,
    pub paid_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AbanClient {
    pub fn new(api_token: String, api_base_url: String) -> Result<Self> {
        let http = Client::builder()
            .user_agent("LoghmehBot/0.1 (payment integration)")
            .build()
            .context("could not create AbanGateway HTTP client")?;
        Ok(Self {
            http,
            api_token,
            api_base_url,
        })
    }

    pub async fn create_invoice(
        &self,
        amount_rial: i64,
        public_order_id: &str,
        callback_url: &str,
        description: &str,
    ) -> Result<AbanInvoice> {
        let endpoint = format!("{}/invoices", self.api_base_url);
        let response = self
            .http
            .post(endpoint)
            .bearer_auth(&self.api_token)
            .json(&CreateInvoiceRequest {
                amount_rial,
                order_id: public_order_id,
                callback_url,
                description,
                metadata: serde_json::json!({"source": "loghmeh"}),
                expiry_minutes: 20,
            })
            .send()
            .await
            .context("could not reach AbanGateway to create the invoice")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("could not read AbanGateway response")?;
        if !status.is_success() {
            bail!("AbanGateway invoice creation failed ({status}): {body}");
        }
        serde_json::from_str(&body).context("AbanGateway returned an invalid invoice response")
    }

    pub async fn verify_invoice(&self, invoice_id: &str) -> Result<AbanVerification> {
        let endpoint = format!("{}/invoices/{invoice_id}/verify", self.api_base_url);
        let response = self
            .http
            .post(endpoint)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .context("could not reach AbanGateway to verify the invoice")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("could not read AbanGateway verification response")?;
        if !status.is_success() {
            bail!("AbanGateway invoice verification failed ({status}): {body}");
        }
        serde_json::from_str(&body).context("AbanGateway returned an invalid verification response")
    }
}
