use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use tnt_delivery_bot::bot::{AppState, build_handler};
use tnt_delivery_bot::config::Config;
use tnt_delivery_bot::db::spawn_db_actor;
use tnt_delivery_bot::fetch::ReqwestFetcher;
use tnt_delivery_bot::notify::{Notifier, TeloxideNotifier};
use tnt_delivery_bot::tick::run_tick_round;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    simple_logger::init_with_env().ok();

    let cfg = Arc::new(Config::from_env()?);
    log::info!(
        "starting: tick={}s N={} ttl={}h max_fail={}",
        cfg.tick_seconds,
        cfg.poll_period_ticks,
        cfg.token_ttl_hours,
        cfg.max_fetch_failures
    );

    let db = spawn_db_actor(&cfg.db_path)?;
    let fetcher: Arc<dyn tnt_delivery_bot::fetch::ApiFetcher> =
        Arc::new(ReqwestFetcher::new(&cfg)?);
    let bot = Bot::new(cfg.bot_token.clone());

    let cancel = CancellationToken::new();

    // Tick loop.
    let tick_handle = {
        let cfg = cfg.clone();
        let db = db.clone();
        let fetcher = fetcher.clone();
        let notifier = TeloxideNotifier::new(bot.clone());
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            let mut interval = tokio::time::interval(Duration::from_secs(cfg.tick_seconds));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = interval.tick() => {
                        tick = tick.wrapping_add(1);
                        if let Err(e) = run_tick_round(
                            tick, &cfg, &db, fetcher.as_ref(), &notifier,
                        ).await {
                            log::error!("tick round error: {e}");
                        }
                    }
                }
            }
            log::info!("tick loop stopped");
        })
    };

    let state = AppState {
        db: db.clone(),
        fetcher: fetcher.clone(),
        cfg: cfg.clone(),
        notifier: Arc::new(TeloxideNotifier::new(bot.clone())) as Arc<dyn Notifier>,
    };

    let mut dispatcher = Dispatcher::builder(bot, build_handler())
        .dependencies(dptree::deps![state])
        .default_handler(|_upd| async {})
        .build();
    let shutdown = dispatcher.shutdown_token();

    // Ctrl-C / SIGTERM coordinator.
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            wait_for_signal().await;
            log::info!("shutdown signal received");
            cancel.cancel();
            if let Err(e) = shutdown.shutdown() {
                log::warn!("shutdown trigger failed: {e:?}");
            }
        });
    }

    dispatcher.dispatch().await;
    cancel.cancel();
    let _ = tick_handle.await;
    Ok(())
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
