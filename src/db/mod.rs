use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Row, postgres::PgPoolOptions, types::Json};
use uuid::Uuid;

use crate::{
    fulfillment::FulfillmentResult,
    payment::{AbanInvoice, AbanVerification},
};

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

#[derive(Clone, Debug, Serialize, FromRow)]
pub struct ServiceOffering {
    pub id: Uuid,
    pub code: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub price_rial: i64,
    pub configuration: Json<serde_json::Value>,
    pub cost_cap_nano_ton: Option<i64>,
    pub is_active: bool,
    pub sort_order: i32,
}

#[derive(Clone, Debug)]
pub struct UserRecord {
    pub id: Uuid,
}

#[derive(Clone, Debug)]
pub struct CreatedOrder {
    pub id: Uuid,
    pub public_id: String,
    pub amount_rial: i64,
    pub title: String,
    pub target_username: String,
}

#[derive(Clone, Debug, Serialize, FromRow)]
pub struct AdminOrderView {
    pub public_id: String,
    pub service_title: String,
    pub target_username: String,
    pub amount_rial: i64,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub username: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DashboardSummary {
    pub active_offerings: i64,
    pub orders_awaiting_payment: i64,
    pub jobs_needing_attention: i64,
    pub paid_today_rial: i64,
}

#[derive(Clone, Debug)]
pub struct FulfillmentWorkItem {
    pub job_id: Uuid,
    pub order_id: Uuid,
    pub public_order_id: String,
    pub target_username: String,
    pub offering_snapshot: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NewOffering {
    pub code: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub price_rial: i64,
    #[serde(default)]
    pub configuration: serde_json::Value,
    pub cost_cap_nano_ton: Option<i64>,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(FromRow)]
struct CheckoutRow {
    user_id: Uuid,
    offering_id: Uuid,
    target_username: String,
    code: String,
    kind: String,
    title: String,
    description: String,
    price_rial: i64,
    configuration: Json<serde_json::Value>,
    cost_cap_nano_ton: Option<i64>,
}

#[derive(FromRow)]
struct ClaimedJobRow {
    job_id: Uuid,
    order_id: Uuid,
    public_order_id: String,
    target_username: String,
    offering_snapshot: Json<serde_json::Value>,
}

impl Database {
    pub async fn connect_and_migrate(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(12)
            .connect(database_url)
            .await
            .context("could not connect to PostgreSQL")?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .context("could not run PostgreSQL migrations")?;
        Ok(Self { pool })
    }

    pub async fn upsert_telegram_user(
        &self,
        telegram_id: i64,
        username: Option<&str>,
        first_name: &str,
        is_admin: bool,
    ) -> Result<UserRecord> {
        let role = if is_admin { "admin" } else { "customer" };
        let row = sqlx::query(
            r#"
            INSERT INTO users (id, telegram_id, username, first_name, role)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (telegram_id) DO UPDATE
                SET username = EXCLUDED.username,
                    first_name = EXCLUDED.first_name,
                    role = CASE WHEN users.role = 'admin' THEN 'admin' ELSE EXCLUDED.role END
            RETURNING id
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(telegram_id)
        .bind(username)
        .bind(first_name)
        .bind(role)
        .fetch_one(&self.pool)
        .await
        .context("could not upsert Telegram user")?;
        Ok(UserRecord {
            id: row.try_get("id")?,
        })
    }

    pub async fn active_offerings(&self) -> Result<Vec<ServiceOffering>> {
        sqlx::query_as(
            r#"
            SELECT id, code, kind, title, description, price_rial, configuration,
                   cost_cap_nano_ton, is_active, sort_order
            FROM service_offerings
            WHERE is_active = TRUE
            ORDER BY sort_order ASC, title ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("could not list active offerings")
    }

    pub async fn create_checkout(
        &self,
        user_id: Uuid,
        offering_id: Uuid,
        target_username: &str,
    ) -> Result<Uuid> {
        let checkout_id = Uuid::new_v4();
        let inserted = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO checkout_sessions (id, user_id, offering_id, target_username, expires_at)
            SELECT $1, $2, $3, $4, now() + interval '15 minutes'
            WHERE EXISTS (
                SELECT 1 FROM service_offerings WHERE id = $3 AND is_active = TRUE
            )
            RETURNING id
            "#,
        )
        .bind(checkout_id)
        .bind(user_id)
        .bind(offering_id)
        .bind(target_username)
        .fetch_optional(&self.pool)
        .await
        .context("could not create checkout session")?;
        inserted.context("this service is no longer available")
    }

    pub async fn cancel_checkout(&self, checkout_id: Uuid, user_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM checkout_sessions WHERE id = $1 AND user_id = $2")
            .bind(checkout_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("could not cancel checkout")?;
        Ok(())
    }

    pub async fn finalize_checkout(
        &self,
        checkout_id: Uuid,
        user_id: Uuid,
    ) -> Result<CreatedOrder> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("could not start checkout transaction")?;
        let checkout: CheckoutRow = sqlx::query_as(
            r#"
            SELECT cs.user_id, cs.offering_id, cs.target_username,
                   so.code, so.kind, so.title, so.description, so.price_rial,
                   so.configuration, so.cost_cap_nano_ton
            FROM checkout_sessions cs
            JOIN service_offerings so ON so.id = cs.offering_id
            WHERE cs.id = $1 AND cs.user_id = $2 AND cs.expires_at > now() AND so.is_active = TRUE
            FOR UPDATE OF cs
            "#,
        )
        .bind(checkout_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .context("could not read checkout session")?
        .context("checkout expired, was already used, or service is unavailable")?;

        let public_id = format!(
            "LGM-{}",
            Uuid::new_v4().simple().to_string()[..12].to_uppercase()
        );
        let order_id = Uuid::new_v4();
        let snapshot = serde_json::json!({
            "offering_id": checkout.offering_id,
            "code": checkout.code,
            "kind": checkout.kind,
            "title": checkout.title,
            "description": checkout.description,
            "configuration": checkout.configuration.0,
            "cost_cap_nano_ton": checkout.cost_cap_nano_ton,
        });
        sqlx::query(
            r#"
            INSERT INTO orders (
                id, public_id, user_id, offering_id, offering_snapshot,
                target_username, amount_rial, status
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'payment_creating')
            "#,
        )
        .bind(order_id)
        .bind(&public_id)
        .bind(checkout.user_id)
        .bind(checkout.offering_id)
        .bind(Json(snapshot))
        .bind(&checkout.target_username)
        .bind(checkout.price_rial)
        .execute(&mut *tx)
        .await
        .context("could not create order")?;
        sqlx::query("DELETE FROM checkout_sessions WHERE id = $1")
            .bind(checkout_id)
            .execute(&mut *tx)
            .await
            .context("could not consume checkout session")?;
        self.add_event(
            &mut tx,
            order_id,
            "customer",
            Some(checkout.user_id.to_string()),
            "order.created",
            serde_json::json!({}),
        )
        .await?;
        tx.commit()
            .await
            .context("could not commit checkout transaction")?;

        Ok(CreatedOrder {
            id: order_id,
            public_id,
            amount_rial: checkout.price_rial,
            title: checkout.title,
            target_username: checkout.target_username,
        })
    }

    pub async fn register_aban_invoice(
        &self,
        order: &CreatedOrder,
        invoice: &AbanInvoice,
    ) -> Result<()> {
        if invoice.amount_rial != order.amount_rial {
            bail!("AbanGateway returned a mismatched invoice amount");
        }
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE orders SET status = 'awaiting_payment', payment_expires_at = $2 WHERE id = $1 AND status = 'payment_creating'",
        )
        .bind(order.id)
        .bind(invoice.expires_at)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            bail!("order was not waiting for payment creation");
        }
        sqlx::query(
            r#"
            INSERT INTO payments (id, order_id, provider, external_id, status, amount_rial, payment_url, provider_payload)
            VALUES ($1, $2, 'aban', $3, $4, $5, $6, $7)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(order.id)
        .bind(&invoice.invoice_id)
        .bind(&invoice.status)
        .bind(invoice.amount_rial)
        .bind(&invoice.payment_url)
        .bind(Json(serde_json::json!({
            "payable_rial": invoice.payable_rial,
            "expires_at": invoice.expires_at,
        })))
        .execute(&mut *tx)
        .await?;
        self.add_event(
            &mut tx,
            order.id,
            "system",
            None,
            "payment.invoice_created",
            serde_json::json!({"provider": "aban", "invoice_id": invoice.invoice_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn mark_payment_creation_failed(&self, order_id: Uuid, reason: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE orders SET status = 'failed' WHERE id = $1 AND status = 'payment_creating'",
        )
        .bind(order_id)
        .execute(&mut *tx)
        .await?;
        self.add_event(
            &mut tx,
            order_id,
            "system",
            None,
            "payment.invoice_creation_failed",
            serde_json::json!({"reason": reason}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn record_webhook(
        &self,
        provider: &str,
        delivery_id: &str,
        event_type: &str,
        signature_verified: bool,
        payload: serde_json::Value,
    ) -> Result<bool> {
        let delivery = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO webhook_deliveries (id, provider, delivery_id, event_type, signature_verified, payload)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (provider, delivery_id) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(provider)
        .bind(delivery_id)
        .bind(event_type)
        .bind(signature_verified)
        .bind(Json(payload))
        .fetch_optional(&self.pool)
        .await
        .context("could not record webhook delivery")?;
        Ok(delivery.is_some())
    }

    pub async fn settle_aban_payment(
        &self,
        verification: &AbanVerification,
        fulfillment_provider: &str,
    ) -> Result<Option<String>> {
        if !verification.verified {
            bail!("AbanGateway did not verify the invoice");
        }
        let mut tx = self.pool.begin().await?;
        let payment = sqlx::query(
            r#"
            SELECT p.order_id, p.status AS payment_status, o.amount_rial, o.public_id, o.status AS order_status
            FROM payments p JOIN orders o ON o.id = p.order_id
            WHERE p.provider = 'aban' AND p.external_id = $1
            FOR UPDATE OF p, o
            "#,
        )
        .bind(&verification.invoice_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("payment invoice is not registered in Loghmeh")?;
        let expected_amount: i64 = payment.try_get("amount_rial")?;
        if expected_amount != verification.amount_rial {
            bail!("verified payment amount does not match the order amount");
        }
        let order_id: Uuid = payment.try_get("order_id")?;
        let public_id: String = payment.try_get("public_id")?;
        let order_status: String = payment.try_get("order_status")?;
        if order_status != "awaiting_payment" {
            tx.commit().await?;
            return Ok(None);
        }

        sqlx::query("UPDATE payments SET status = 'paid', verified_at = now() WHERE provider = 'aban' AND external_id = $1")
            .bind(&verification.invoice_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE orders SET status = 'paid', paid_at = COALESCE($2, now()) WHERE id = $1",
        )
        .bind(order_id)
        .bind(verification.paid_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO fulfillment_jobs (id, order_id, provider, status)
            VALUES ($1, $2, $3, 'queued')
            ON CONFLICT (order_id) DO NOTHING
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(order_id)
        .bind(fulfillment_provider)
        .execute(&mut *tx)
        .await?;
        self.add_event(&mut tx, order_id, "payment_provider", Some("aban".into()), "payment.verified", serde_json::json!({"invoice_id": verification.invoice_id, "order_id": verification.order_id}))
            .await?;
        tx.commit().await?;
        Ok(Some(public_id))
    }

    pub async fn claim_next_fulfillment_job(
        &self,
        worker_id: &str,
    ) -> Result<Option<FulfillmentWorkItem>> {
        let mut tx = self.pool.begin().await?;
        let job_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            WITH candidate AS (
                SELECT id FROM fulfillment_jobs
                WHERE status IN ('queued', 'retryable_failure')
                  AND next_attempt_at <= now()
                  AND (locked_at IS NULL OR locked_at < now() - interval '10 minutes')
                ORDER BY next_attempt_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            UPDATE fulfillment_jobs job
            SET status = 'processing', locked_at = now(), locked_by = $1,
                attempt_count = attempt_count + 1
            FROM candidate
            WHERE job.id = candidate.id
            RETURNING job.id
            "#,
        )
        .bind(worker_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(job_id) = job_id else {
            tx.commit().await?;
            return Ok(None);
        };
        let job: ClaimedJobRow = sqlx::query_as(
            r#"
            SELECT job.id AS job_id, ord.id AS order_id, ord.public_id AS public_order_id,
                   ord.target_username, ord.offering_snapshot
            FROM fulfillment_jobs job
            JOIN orders ord ON ord.id = job.order_id
            WHERE job.id = $1
            "#,
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE orders SET status = 'fulfillment_processing' WHERE id = $1 AND status = 'paid'",
        )
        .bind(job.order_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(FulfillmentWorkItem {
            job_id: job.job_id,
            order_id: job.order_id,
            public_order_id: job.public_order_id,
            target_username: job.target_username,
            offering_snapshot: job.offering_snapshot.0,
        }))
    }

    pub async fn finish_fulfillment_job(
        &self,
        job_id: Uuid,
        result: FulfillmentResult,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT order_id, attempt_count FROM fulfillment_jobs WHERE id = $1 FOR UPDATE",
        )
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("fulfillment job not found")?;
        let order_id: Uuid = row.try_get("order_id")?;
        let attempt_count: i32 = row.try_get("attempt_count")?;
        let (job_status, order_status, event_type, reason, reference, retry_after_seconds) =
            match result {
                FulfillmentResult::Succeeded { reference } => (
                    "succeeded",
                    "fulfilled",
                    "fulfillment.succeeded",
                    None,
                    Some(reference),
                    None,
                ),
                FulfillmentResult::ManualReview { reason } => (
                    "manual_review",
                    "requires_review",
                    "fulfillment.manual_review",
                    Some(reason),
                    None,
                    None,
                ),
                FulfillmentResult::PermanentFailure { reason } => (
                    "failed",
                    "failed",
                    "fulfillment.failed",
                    Some(reason),
                    None,
                    None,
                ),
                FulfillmentResult::RetryableFailure { reason } if attempt_count >= 3 => (
                    "manual_review",
                    "requires_review",
                    "fulfillment.retry_exhausted",
                    Some(reason),
                    None,
                    None,
                ),
                FulfillmentResult::RetryableFailure { reason } => (
                    "retryable_failure",
                    "paid",
                    "fulfillment.retryable_failure",
                    Some(reason),
                    None,
                    Some(30_i64 * attempt_count as i64),
                ),
            };
        sqlx::query(
            r#"
            UPDATE fulfillment_jobs
            SET status = $2, last_error = $3, provider_reference = $4,
                next_attempt_at = CASE WHEN $5::BIGINT IS NULL THEN next_attempt_at
                    ELSE now() + (($5::TEXT || ' seconds')::interval) END,
                locked_at = NULL, locked_by = NULL
            WHERE id = $1
            "#,
        )
        .bind(job_id)
        .bind(job_status)
        .bind(reason.as_deref())
        .bind(reference.as_deref())
        .bind(retry_after_seconds)
        .execute(&mut *tx)
        .await?;
        if job_status != "retryable_failure" {
            sqlx::query("UPDATE orders SET status = $2, fulfilled_at = CASE WHEN $2 = 'fulfilled' THEN now() ELSE fulfilled_at END WHERE id = $1")
                .bind(order_id)
                .bind(order_status)
                .execute(&mut *tx)
                .await?;
        }
        self.add_event(
            &mut tx,
            order_id,
            "fulfillment_provider",
            None,
            event_type,
            serde_json::json!({"reason": reason, "reference": reference}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn dashboard_summary(&self) -> Result<DashboardSummary> {
        let row = sqlx::query(
            r#"
            SELECT
              (SELECT count(*) FROM service_offerings WHERE is_active) AS active_offerings,
              (SELECT count(*) FROM orders WHERE status = 'awaiting_payment') AS awaiting_payment,
              (SELECT count(*) FROM fulfillment_jobs WHERE status IN ('manual_review', 'failed')) AS needs_attention,
              (SELECT COALESCE(sum(amount_rial), 0) FROM orders WHERE paid_at >= date_trunc('day', now())) AS paid_today_rial
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(DashboardSummary {
            active_offerings: row.try_get("active_offerings")?,
            orders_awaiting_payment: row.try_get("awaiting_payment")?,
            jobs_needing_attention: row.try_get("needs_attention")?,
            paid_today_rial: row.try_get("paid_today_rial")?,
        })
    }

    pub async fn list_recent_orders(&self, limit: i64) -> Result<Vec<AdminOrderView>> {
        sqlx::query_as(
            r#"
            SELECT o.public_id,
                   o.offering_snapshot->>'title' AS service_title,
                   o.target_username, o.amount_rial, o.status, o.created_at, u.username
            FROM orders o JOIN users u ON u.id = o.user_id
            ORDER BY o.created_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await
        .context("could not list recent orders")
    }

    pub async fn list_user_orders(&self, user_id: Uuid) -> Result<Vec<AdminOrderView>> {
        sqlx::query_as(
            r#"
            SELECT o.public_id,
                   o.offering_snapshot->>'title' AS service_title,
                   o.target_username, o.amount_rial, o.status, o.created_at, u.username
            FROM orders o JOIN users u ON u.id = o.user_id
            WHERE o.user_id = $1
            ORDER BY o.created_at DESC
            LIMIT 10
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .context("could not list user orders")
    }

    pub async fn all_offerings(&self) -> Result<Vec<ServiceOffering>> {
        sqlx::query_as(
            r#"
            SELECT id, code, kind, title, description, price_rial, configuration,
                   cost_cap_nano_ton, is_active, sort_order
            FROM service_offerings ORDER BY sort_order ASC, title ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("could not list offerings")
    }

    pub async fn create_offering(&self, offering: NewOffering) -> Result<ServiceOffering> {
        validate_offering(&offering)?;
        sqlx::query_as(
            r#"
            INSERT INTO service_offerings (
                id, code, kind, title, description, price_rial, configuration,
                cost_cap_nano_ton, is_active, sort_order
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, code, kind, title, description, price_rial, configuration,
                      cost_cap_nano_ton, is_active, sort_order
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(offering.code.trim().to_ascii_lowercase())
        .bind(offering.kind)
        .bind(offering.title.trim())
        .bind(offering.description.trim())
        .bind(offering.price_rial)
        .bind(Json(offering.configuration))
        .bind(offering.cost_cap_nano_ton)
        .bind(offering.is_active)
        .bind(offering.sort_order)
        .fetch_one(&self.pool)
        .await
        .context("could not create offering")
    }

    pub async fn toggle_offering(&self, offering_id: Uuid) -> Result<ServiceOffering> {
        sqlx::query_as(
            r#"
            UPDATE service_offerings SET is_active = NOT is_active
            WHERE id = $1
            RETURNING id, code, kind, title, description, price_rial, configuration,
                      cost_cap_nano_ton, is_active, sort_order
            "#,
        )
        .bind(offering_id)
        .fetch_optional(&self.pool)
        .await?
        .context("offering not found")
    }

    pub async fn log_admin_action(
        &self,
        source: &str,
        action: &str,
        subject_type: &str,
        subject_id: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO admin_audit_logs (id, source, action, subject_type, subject_id, payload)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(source)
        .bind(action)
        .bind(subject_type)
        .bind(subject_id)
        .bind(Json(payload))
        .execute(&self.pool)
        .await
        .context("could not write admin audit log")?;
        Ok(())
    }

    async fn add_event(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        order_id: Uuid,
        actor_type: &str,
        actor_id: Option<String>,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO order_events (id, order_id, actor_type, actor_id, event_type, payload)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(order_id)
        .bind(actor_type)
        .bind(actor_id)
        .bind(event_type)
        .bind(Json(payload))
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

fn validate_offering(offering: &NewOffering) -> Result<()> {
    if offering.code.trim().is_empty() || offering.code.len() > 64 {
        bail!("offering code must be 1 to 64 characters");
    }
    if offering.title.trim().is_empty() || offering.title.chars().count() > 80 {
        bail!("offering title must be 1 to 80 characters");
    }
    if !matches!(
        offering.kind.as_str(),
        "telegram_premium" | "telegram_stars" | "manual" | "external"
    ) {
        bail!("unsupported offering kind");
    }
    if offering.price_rial <= 0 {
        bail!("price_rial must be positive");
    }
    if offering.cost_cap_nano_ton.is_some_and(|value| value <= 0) {
        bail!("cost_cap_nano_ton must be positive when set");
    }
    Ok(())
}
