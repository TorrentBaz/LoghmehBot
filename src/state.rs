use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    config::Config, db::Database, fulfillment::DynFulfillmentProvider, payment::AbanClient,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Database,
    pub aban: AbanClient,
    pub fulfillment: DynFulfillmentProvider,
    pub pending_offerings: Arc<Mutex<HashMap<i64, Uuid>>>,
}

impl AppState {
    pub async fn select_offering(&self, telegram_id: i64, offering_id: Uuid) {
        self.pending_offerings
            .lock()
            .await
            .insert(telegram_id, offering_id);
    }

    pub async fn take_selected_offering(&self, telegram_id: i64) -> Option<Uuid> {
        self.pending_offerings.lock().await.remove(&telegram_id)
    }
}
