use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{db::NewOffering, state::AppState};

type HmacSha256 = Hmac<Sha256>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/admin", get(admin_page))
        .route("/webhooks/aban", post(aban_webhook))
        .route("/api/admin/summary", get(admin_summary))
        .route("/api/admin/orders", get(admin_orders))
        .route(
            "/api/admin/offerings",
            get(admin_offerings).post(create_offering),
        )
        .route(
            "/api/admin/offerings/{offering_id}/toggle",
            post(toggle_offering),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn admin_page() -> Html<&'static str> {
    Html(include_str!("../admin/dashboard.html"))
}

async fn admin_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::db::DashboardSummary>, AppError> {
    require_admin(&headers, &state)?;
    Ok(Json(state.db.dashboard_summary().await?))
}

async fn admin_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::db::AdminOrderView>>, AppError> {
    require_admin(&headers, &state)?;
    Ok(Json(state.db.list_recent_orders(50).await?))
}

async fn admin_offerings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::db::ServiceOffering>>, AppError> {
    require_admin(&headers, &state)?;
    Ok(Json(state.db.all_offerings().await?))
}

async fn create_offering(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(offering): Json<NewOffering>,
) -> Result<(StatusCode, Json<crate::db::ServiceOffering>), AppError> {
    require_admin(&headers, &state)?;
    let offering = state.db.create_offering(offering).await?;
    if let Err(error) = state
        .db
        .log_admin_action(
            "web",
            "offering.created",
            "service_offering",
            &offering.id.to_string(),
            serde_json::json!({"code": &offering.code, "active": offering.is_active}),
        )
        .await
    {
        warn!(%error, offering = %offering.id, "offering was created but audit logging failed");
    }
    info!(offering = %offering.id, "admin created offering");
    Ok((StatusCode::CREATED, Json(offering)))
}

async fn toggle_offering(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(offering_id): Path<Uuid>,
) -> Result<Json<crate::db::ServiceOffering>, AppError> {
    require_admin(&headers, &state)?;
    let offering = state.db.toggle_offering(offering_id).await?;
    if let Err(error) = state
        .db
        .log_admin_action(
            "web",
            "offering.toggled",
            "service_offering",
            &offering.id.to_string(),
            serde_json::json!({"active": offering.is_active}),
        )
        .await
    {
        warn!(%error, offering = %offering.id, "offering was toggled but audit logging failed");
    }
    info!(offering = %offering.id, active = offering.is_active, "admin toggled offering");
    Ok(Json(offering))
}

#[derive(Deserialize)]
struct AbanWebhook {
    event: String,
    invoice_id: String,
}

async fn aban_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let delivery_id = header(&headers, "X-Delivery-Id")?;
    let event_header = header(&headers, "X-Event")?;
    let signature = header(&headers, "X-Signature")?;
    if !verify_signature(&state.config.aban_webhook_secret, &body, signature) {
        warn!(
            delivery_id,
            "rejected AbanGateway webhook with invalid signature"
        );
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid webhook signature",
        ));
    }
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "invalid JSON payload"))?;
    let event: AbanWebhook = serde_json::from_value(payload.clone())
        .map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "invalid AbanGateway payload"))?;
    if event.event != event_header {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "event header does not match payload",
        ));
    }
    let is_new = state
        .db
        .record_webhook("aban", delivery_id, &event.event, true, payload)
        .await?;
    if !is_new {
        return Ok(StatusCode::OK);
    }
    if event.event != "invoice.paid" {
        return Ok(StatusCode::OK);
    }
    let verification = state.aban.verify_invoice(&event.invoice_id).await?;
    let fulfillment_provider = state.fulfillment.name();
    if let Some(order) = state
        .db
        .settle_aban_payment(&verification, fulfillment_provider)
        .await?
    {
        info!(%order, invoice = %verification.invoice_id, "payment verified and fulfillment queued");
    }
    Ok(StatusCode::OK)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, AppError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("missing or invalid {name} header"),
            )
        })
}

fn verify_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let Ok(signature) = hex::decode(signature) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

fn require_admin(headers: &HeaderMap, state: &AppState) -> Result<(), AppError> {
    let expected = format!("Bearer {}", state.config.admin_api_token);
    let supplied = headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if supplied != expected {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "admin authorization required",
        ));
    }
    Ok(())
}

pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        error!(%error, "HTTP request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::verify_signature;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    #[test]
    fn verifies_raw_webhook_payload() {
        let body = br#"{\"event\":\"invoice.paid\"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(body);
        assert!(verify_signature(
            "secret",
            body,
            &hex::encode(mac.finalize().into_bytes())
        ));
        assert!(!verify_signature("wrong", body, "00"));
    }
}
