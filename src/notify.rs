use async_trait::async_trait;

#[derive(Debug)]
pub enum NotifyError {
    /// User blocked the bot / deactivated — drop their subscription.
    Forbidden,
    Other(String),
}

impl NotifyError {
    pub fn is_forbidden(&self) -> bool {
        matches!(self, NotifyError::Forbidden)
    }
}

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn send(&self, chat_id: i64, text: &str) -> Result<(), NotifyError>;
}

pub struct TeloxideNotifier {
    bot: teloxide::Bot,
}

impl TeloxideNotifier {
    pub fn new(bot: teloxide::Bot) -> Self {
        Self { bot }
    }
}

#[async_trait]
impl Notifier for TeloxideNotifier {
    async fn send(&self, chat_id: i64, text: &str) -> Result<(), NotifyError> {
        use teloxide::ApiError;
        use teloxide::RequestError;
        use teloxide::prelude::*;
        match self
            .bot
            .send_message(teloxide::types::ChatId(chat_id), text)
            .await
        {
            Ok(_) => Ok(()),
            Err(RequestError::Api(ApiError::BotBlocked))
            | Err(RequestError::Api(ApiError::UserDeactivated))
            | Err(RequestError::Api(ApiError::ChatNotFound))
            | Err(RequestError::Api(ApiError::CantTalkWithBots)) => Err(NotifyError::Forbidden),
            Err(e) => Err(NotifyError::Other(e.to_string())),
        }
    }
}

#[cfg(any(test, feature = "test-fakes"))]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    pub struct FakeNotifier {
        pub sent: Mutex<Vec<(i64, String)>>,
        pub forbidden_chats: Mutex<Vec<i64>>,
    }

    impl FakeNotifier {
        pub fn new() -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                forbidden_chats: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Notifier for FakeNotifier {
        async fn send(&self, chat_id: i64, text: &str) -> Result<(), NotifyError> {
            if self.forbidden_chats.lock().unwrap().contains(&chat_id) {
                return Err(NotifyError::Forbidden);
            }
            self.sent.lock().unwrap().push((chat_id, text.to_string()));
            Ok(())
        }
    }
}
