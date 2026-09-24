use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::config::{Config, FulfillmentProviderKind};

pub mod worker;

pub type DynFulfillmentProvider = Arc<dyn FulfillmentProvider>;

#[derive(Clone, Debug)]
pub struct FulfillmentRequest {
    pub order_id: Uuid,
    pub public_order_id: String,
    pub target_username: String,
    pub offering_snapshot: serde_json::Value,
}

#[derive(Clone, Debug)]
pub enum FulfillmentResult {
    Succeeded { reference: String },
    RetryableFailure { reason: String },
    ManualReview { reason: String },
    PermanentFailure { reason: String },
}

#[async_trait]
pub trait FulfillmentProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn fulfill(&self, request: FulfillmentRequest) -> Result<FulfillmentResult>;
}

pub fn build_provider(config: &Config) -> Result<DynFulfillmentProvider> {
    match config.fulfillment_provider {
        FulfillmentProviderKind::Manual => Ok(Arc::new(ManualProvider)),
        FulfillmentProviderKind::FragmentConnector => Ok(Arc::new(ExternalProvider::new(
            config
                .fulfillment_endpoint
                .clone()
                .expect("validated in config"),
            config
                .fulfillment_hmac_secret
                .clone()
                .expect("validated in config"),
        )?)),
    }
}

struct ManualProvider;

#[async_trait]
impl FulfillmentProvider for ManualProvider {
    fn name(&self) -> &'static str {
        "manual"
    }

    async fn fulfill(&self, request: FulfillmentRequest) -> Result<FulfillmentResult> {
        Ok(FulfillmentResult::ManualReview {
            reason: format!(
                "Order {} for {} awaits an operator. Automatic delivery is intentionally disabled.",
                request.public_order_id, request.target_username
            ),
        })
    }
}

struct ExternalProvider {
    http: Client,
    endpoint: String,
    hmac_secret: String,
}

#[derive(Serialize)]
struct ExternalRequest<'a> {
    order_id: Uuid,
    public_order_id: &'a str,
    target_username: &'a str,
    offering: &'a serde_json::Value,
}

#[derive(Deserialize)]
struct ExternalResponse {
    status: String,
    reference: Option<String>,
    message: Option<String>,
}

impl ExternalProvider {
    fn new(endpoint: String, hmac_secret: String) -> Result<Self> {
        let http = Client::builder()
            .user_agent("LoghmehBot/0.1 (fulfillment connector)")
            .build()
            .context("could not create fulfillment HTTP client")?;
        Ok(Self {
            http,
            endpoint,
            hmac_secret,
        })
    }
}

#[async_trait]
impl FulfillmentProvider for ExternalProvider {
    fn name(&self) -> &'static str {
        "external"
    }

    async fn fulfill(&self, request: FulfillmentRequest) -> Result<FulfillmentResult> {
        let payload = ExternalRequest {
            order_id: request.order_id,
            public_order_id: &request.public_order_id,
            target_username: &request.target_username,
            offering: &request.offering_snapshot,
        };
        let body =
            serde_json::to_vec(&payload).context("could not serialize fulfillment request")?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.hmac_secret.as_bytes())
            .expect("HMAC accepts keys of any size");
        mac.update(&body);
        let signature = hex::encode(mac.finalize().into_bytes());

        let response = self
            .http
            .post(&self.endpoint)
            .header("X-Loghmeh-Signature", signature)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .context("fulfillment connector could not be reached")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("could not read fulfillment connector response")?;
        if !status.is_success() {
            return Ok(FulfillmentResult::RetryableFailure {
                reason: format!("connector returned {status}: {body}"),
            });
        }
        let payload: ExternalResponse =
            serde_json::from_str(&body).context("fulfillment connector returned invalid JSON")?;
        let reason = payload
            .message
            .unwrap_or_else(|| "connector did not include a message".into());
        match payload.status.as_str() {
            "succeeded" => Ok(FulfillmentResult::Succeeded {
                reference: payload.reference.unwrap_or_else(|| request.public_order_id),
            }),
            "retryable_failure" => Ok(FulfillmentResult::RetryableFailure { reason }),
            "manual_review" => Ok(FulfillmentResult::ManualReview { reason }),
            "permanent_failure" => Ok(FulfillmentResult::PermanentFailure { reason }),
            other => bail!("fulfillment connector returned unknown status: {other}"),
        }
    }
}
