mod admin;
mod config;
mod db;
mod fulfillment;
mod payment;
mod state;
mod telegram;
mod web;

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use teloxide::prelude::*;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::{config::Config, fulfillment::build_provider, payment::AbanClient, state::AppState};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let config = Arc::new(Config::from_env()?);
    let db = db::Database::connect_and_migrate(&config.database_url).await?;
    let aban = AbanClient::new(
        config.aban_api_token.clone(),
        config.aban_api_base_url.clone(),
    )?;
    let fulfillment = build_provider(&config)?;
    let bot = Bot::new(&config.telegram_bot_token);
    let state = AppState {
        config: config.clone(),
        db,
        aban,
        fulfillment,
        pending_offerings: Arc::new(Mutex::new(HashMap::new())),
    };

    let worker_state = state.clone();
    tokio::spawn(async move { fulfillment::worker::run(worker_state).await });

    let listener = tokio::net::TcpListener::bind(config.bind_address)
        .await
        .with_context(|| format!("could not bind HTTP server to {}", config.bind_address))?;
    info!(address = %config.bind_address, "Loghmeh is running");

    let web_server = axum::serve(listener, web::router(state.clone()));
    let dispatcher = Dispatcher::builder(bot, telegram::schema())
        .dependencies(teloxide::dptree::deps![state])
        .enable_ctrlc_handler()
        .build();

    tokio::select! {
        result = web_server => result.context("HTTP server stopped unexpectedly"),
        _ = dispatcher.dispatch() => Ok(()),
    }
}
