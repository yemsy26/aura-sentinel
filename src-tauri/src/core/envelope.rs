use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEnvelope<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<EnvelopeError>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeError {
    pub code: EnvelopeErrorCode,
    pub message: String,
    pub hint: Option<String>,
    pub retryable: bool,
    pub requires_human: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvelopeErrorCode {
    Timeout,
    RateLimit,
    SyntaxError,
    CompilationError,
    NotFound,
    PermissionDenied,
    MissingCredential,
    PolicyViolation,
    CommandFailed,
    SchemaIncompatible,
    ConsecutiveFailureLimit,
    Unknown,
}

impl<T> ToolEnvelope<T> {
    #[allow(dead_code)]
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    #[allow(dead_code)]
    pub fn failure(
        code: EnvelopeErrorCode,
        message: &str,
        hint: Option<String>,
        retryable: bool,
        requires_human: bool,
    ) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(EnvelopeError {
                code,
                message: message.to_string(),
                hint,
                retryable,
                requires_human,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_success() {
        let env = ToolEnvelope::success("created file".to_string());
        assert!(env.ok);
        assert_eq!(env.data.unwrap(), "created file");
        assert!(env.error.is_none());
    }

    #[test]
    fn test_envelope_failure() {
        let env: ToolEnvelope<String> = ToolEnvelope::failure(
            EnvelopeErrorCode::SyntaxError,
            "Expected semicolon",
            Some("Add semicolon at end of line".to_string()),
            true,
            false,
        );
        assert!(!env.ok);
        assert!(env.error.is_some());
        let err = env.error.unwrap();
        assert_eq!(err.code, EnvelopeErrorCode::SyntaxError);
        assert!(err.retryable);
        assert!(!err.requires_human);
    }
}
