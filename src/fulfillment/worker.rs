use std::time::Duration;

use anyhow::Result;
use tracing::{error, info};

use crate::{fulfillment::FulfillmentResult, state::AppState};

pub async fn run(state: AppState) {
    let worker_id = format!("worker-{}", uuid::Uuid::new_v4().simple());
    loop {
        match state.db.claim_next_fulfillment_job(&worker_id).await {
            Ok(Some(job)) => {
                info!(order = %job.public_order_id, provider = state.fulfillment.name(), "fulfillment job claimed");
                let result = state
                    .fulfillment
                    .fulfill(crate::fulfillment::FulfillmentRequest {
                        order_id: job.order_id,
                        public_order_id: job.public_order_id,
                        target_username: job.target_username,
                        offering_snapshot: job.offering_snapshot,
                    })
                    .await;
                match result {
                    Ok(result) => {
                        if let Err(error) =
                            state.db.finish_fulfillment_job(job.job_id, result).await
                        {
                            error!(%error, "could not finish fulfillment job");
                        }
                    }
                    Err(error) => {
                        error!(%error, "fulfillment provider failed unexpectedly");
                        let result = FulfillmentResult::RetryableFailure {
                            reason: error.to_string(),
                        };
                        if let Err(error) =
                            state.db.finish_fulfillment_job(job.job_id, result).await
                        {
                            error!(%error, "could not persist fulfillment provider failure");
                        }
                    }
                }
            }
            Ok(None) => tokio::time::sleep(Duration::from_secs(3)).await,
            Err(error) => {
                error!(%error, "could not poll fulfillment queue");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
