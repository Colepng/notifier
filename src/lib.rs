use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[must_use]
pub struct NotificationWrapper {
    pub text: String,
}

impl NotificationWrapper {
    pub const fn new(text: String) -> Self {
        Self { 
            text
        }
    }
}

impl From<notify_rust::Notification> for NotificationWrapper {
    fn from(value: notify_rust::Notification) -> Self {
        Self {
            text: value.summary,
        }
    }
}
