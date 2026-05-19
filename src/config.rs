use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub bot_token: String, // secret: never log
    pub db_path: String,
    pub tick_seconds: u64,
    pub poll_period_ticks: u64, // N
    pub token_ttl_hours: i64,
    pub max_fetch_failures: i64,
    pub api_base: String,
    pub http_proxy: Option<String>,
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> Result<T> {
    match std::env::var(key) {
        Ok(v) => v
            .parse::<T>()
            .map_err(|_| anyhow::anyhow!("invalid value for {key}: {v}")),
        Err(_) => Ok(default),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bot_token = std::env::var("BOT_TOKEN")
            .context("BOT_TOKEN is required")?;
        if bot_token.trim().is_empty() {
            anyhow::bail!("BOT_TOKEN is empty");
        }
        let cfg = Config {
            bot_token,
            db_path: std::env::var("DB_PATH")
                .unwrap_or_else(|_| "./tnt-delivery-bot.sqlite".to_string()),
            tick_seconds: parse_env("TICK_SECONDS", 10u64)?,
            poll_period_ticks: parse_env("POLL_PERIOD_TICKS", 12u64)?,
            token_ttl_hours: parse_env("TOKEN_TTL_HOURS", 24i64)?,
            max_fetch_failures: parse_env("MAX_FETCH_FAILURES", 5i64)?,
            api_base: std::env::var("API_BASE").unwrap_or_else(|_| {
                "https://tmsapi.tntsupermarket.us/track/customer".to_string()
            }),
            http_proxy: std::env::var("HTTP_PROXY").ok().filter(|s| !s.is_empty()),
        };
        if cfg.tick_seconds == 0 {
            anyhow::bail!("TICK_SECONDS must be > 0");
        }
        if cfg.poll_period_ticks == 0 {
            anyhow::bail!("POLL_PERIOD_TICKS must be >= 1");
        }
        if cfg.token_ttl_hours < 1 {
            anyhow::bail!("TOKEN_TTL_HOURS must be >= 1");
        }
        if cfg.max_fetch_failures < 1 {
            anyhow::bail!("MAX_FETCH_FAILURES must be >= 1");
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // env tests mutate process env; keep them serialized in one test.
    #[test]
    fn defaults_and_validation() {
        // SAFETY: single-threaded test, no other test touches these vars.
        unsafe {
            std::env::set_var("BOT_TOKEN", "x");
            std::env::remove_var("TICK_SECONDS");
            std::env::remove_var("POLL_PERIOD_TICKS");
        }
        let c = Config::from_env().unwrap();
        assert_eq!(c.tick_seconds, 10);
        assert_eq!(c.poll_period_ticks, 12);
        assert_eq!(c.token_ttl_hours, 24);

        unsafe { std::env::set_var("POLL_PERIOD_TICKS", "0") };
        assert!(Config::from_env().is_err());

        unsafe {
            std::env::set_var("POLL_PERIOD_TICKS", "12");
            std::env::remove_var("BOT_TOKEN");
        }
        assert!(Config::from_env().is_err());
        unsafe { std::env::set_var("BOT_TOKEN", "x") }; // restore for other tests
    }
}
