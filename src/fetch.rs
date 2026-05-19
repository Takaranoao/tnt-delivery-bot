use crate::config::Config;
use crate::model::ApiResponse;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ApiFetcher: Send + Sync {
    async fn fetch(&self, token: &str) -> Result<ApiResponse>;
}

pub struct ReqwestFetcher {
    client: reqwest::Client,
    api_base: String,
}

impl ReqwestFetcher {
    pub fn new(cfg: &Config) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15));
        if let Some(proxy) = &cfg.http_proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        Ok(Self {
            client: builder.build()?,
            api_base: cfg.api_base.clone(),
        })
    }
}

#[async_trait]
impl ApiFetcher for ReqwestFetcher {
    async fn fetch(&self, token: &str) -> Result<ApiResponse> {
        let resp = self
            .client
            .get(&self.api_base)
            .query(&[("token", token)])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<ApiResponse>().await?)
    }
}

#[cfg(any(test, feature = "test-fakes"))]
pub mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Maps token -> queued responses (popped front each call).
    pub struct FakeFetcher {
        pub responses: Mutex<HashMap<String, Vec<Result<ApiResponse, String>>>>,
    }

    impl FakeFetcher {
        pub fn new() -> Self {
            Self { responses: Mutex::new(HashMap::new()) }
        }
        pub fn push_ok(&self, token: &str, json: &str) {
            let r = serde_json::from_str::<ApiResponse>(json).unwrap();
            self.responses
                .lock()
                .unwrap()
                .entry(token.to_string())
                .or_default()
                .push(Ok(r));
        }
        pub fn push_err(&self, token: &str, msg: &str) {
            self.responses
                .lock()
                .unwrap()
                .entry(token.to_string())
                .or_default()
                .push(Err(msg.to_string()));
        }
    }

    #[async_trait]
    impl ApiFetcher for FakeFetcher {
        async fn fetch(&self, token: &str) -> Result<ApiResponse> {
            let mut map = self.responses.lock().unwrap();
            let q = map.get_mut(token).expect("no fake response queued");
            assert!(!q.is_empty(), "fake fetcher queue empty for {token}");
            match q.remove(0) {
                Ok(r) => Ok(r),
                Err(m) => Err(anyhow::anyhow!(m)),
            }
        }
    }
}
