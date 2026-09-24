use std::{collections::HashSet, env, net::SocketAddr, str::FromStr};

use anyhow::{Context, Result, bail};

#[derive(Clone)]
pub struct Config {
    pub telegram_bot_token: String,
    pub database_url: String,
    pub public_base_url: String,
    pub admin_telegram_ids: HashSet<i64>,
    pub admin_api_token: String,
    pub aban_api_token: String,
    pub aban_webhook_secret: String,
    pub aban_api_base_url: String,
    pub fulfillment_provider: FulfillmentProviderKind,
    pub fulfillment_endpoint: Option<String>,
    pub fulfillment_hmac_secret: Option<String>,
    pub bind_address: SocketAddr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FulfillmentProviderKind {
    Manual,
    FragmentConnector,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let telegram_bot_token = required("TELOXIDE_TOKEN")?;
        let database_url = required("DATABASE_URL")?;
        let public_base_url = required("PUBLIC_BASE_URL")?
            .trim_end_matches('/')
            .to_owned();
        let admin_api_token = required("ADMIN_API_TOKEN")?;
        let aban_api_token = required("ABAN_API_TOKEN")?;
        let aban_webhook_secret = required("ABAN_WEBHOOK_SECRET")?;
        let aban_api_base_url = env::var("ABAN_API_BASE_URL")
            .unwrap_or_else(|_| "https://abangateway.ir/api/v1".into())
            .trim_end_matches('/')
            .to_owned();
        let bind_address = env::var("APP_BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0:8080".into())
            .parse()
            .context("APP_BIND_ADDRESS must look like 0.0.0.0:8080")?;
        let admin_telegram_ids = required("ADMIN_TELEGRAM_IDS")?
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                i64::from_str(value).context("ADMIN_TELEGRAM_IDS contains an invalid numeric ID")
            })
            .collect::<Result<HashSet<_>>>()?;
        if admin_telegram_ids.is_empty() {
            bail!("ADMIN_TELEGRAM_IDS must contain at least one numeric Telegram user ID");
        }

        let fulfillment_provider = match env::var("FULFILLMENT_PROVIDER")
            .unwrap_or_else(|_| "manual".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "manual" => FulfillmentProviderKind::Manual,
            "fragment_connector" => FulfillmentProviderKind::FragmentConnector,
            other => {
                bail!("FULFILLMENT_PROVIDER must be manual or fragment_connector, got {other}")
            }
        };
        let fulfillment_endpoint = optional("FULFILLMENT_ENDPOINT");
        let fulfillment_hmac_secret = optional("FULFILLMENT_HMAC_SECRET");
        if fulfillment_provider == FulfillmentProviderKind::FragmentConnector
            && (fulfillment_endpoint.is_none() || fulfillment_hmac_secret.is_none())
        {
            bail!("external fulfillment requires FULFILLMENT_ENDPOINT and FULFILLMENT_HMAC_SECRET");
        }

        Ok(Self {
            telegram_bot_token,
            database_url,
            public_base_url,
            admin_telegram_ids,
            admin_api_token,
            aban_api_token,
            aban_webhook_secret,
            aban_api_base_url,
            fulfillment_provider,
            fulfillment_endpoint,
            fulfillment_hmac_secret,
            bind_address,
        })
    }

    pub fn is_telegram_admin(&self, telegram_id: i64) -> bool {
        self.admin_telegram_ids.contains(&telegram_id)
    }
}

fn required(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() || value.contains("replace_") {
        bail!("{name} must be set to a real value");
    }
    Ok(value)
}

fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
