use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct AppError {
    pub category: &'static str,
    pub message: String,
}

impl AppError {
    pub fn auth(message: impl Into<String>) -> Self {
        Self {
            category: "auth",
            message: message.into(),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            category: "validation",
            message: message.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self {
            category: "config",
            message: message.into(),
        }
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self {
            category: "network",
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            category: "not_found",
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            category: "conflict",
            message: message.into(),
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self.category {
            "validation" | "config" | "not_found" | "conflict" => 2,
            _ => 1,
        }
    }
}

impl From<crate::service::types::ServiceError> for AppError {
    fn from(value: crate::service::types::ServiceError) -> Self {
        Self {
            category: value.category,
            message: value.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_errors_use_exit_code_two() {
        let error = AppError::validation("bad input");

        assert_eq!(error.category, "validation");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn errors_serialize_for_internal_api() {
        let error = AppError::network("timeout");
        let json = serde_json::to_string(&error).expect("error serializes");

        assert!(json.contains("network"));
        assert!(json.contains("timeout"));
    }
}
