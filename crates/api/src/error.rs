use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};

use crate::tawhiri::{rfc3339, ErrorBody, ErrorResponse, Metadata};

/// Tawhiri v1 error types. HTTP status matches the CUSF/SondeHub API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiErrorKind {
    RequestException,
    InvalidDatasetException,
    PredictionException,
    InternalException,
    NotYetImplementedException,
}

impl ApiErrorKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::RequestException => "RequestException",
            Self::InvalidDatasetException => "InvalidDatasetException",
            Self::PredictionException => "PredictionException",
            Self::InternalException => "InternalException",
            Self::NotYetImplementedException => "NotYetImplementedException",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::RequestException => StatusCode::BAD_REQUEST,
            Self::InvalidDatasetException => StatusCode::NOT_FOUND,
            Self::PredictionException | Self::InternalException => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotYetImplementedException => StatusCode::NOT_IMPLEMENTED,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub description: String,
    pub start_datetime: DateTime<Utc>,
    pub complete_datetime: DateTime<Utc>,
}

impl ApiError {
    pub fn new(kind: ApiErrorKind, description: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self {
            kind,
            description: description.into(),
            start_datetime: start,
            complete_datetime: Utc::now(),
        }
    }

    pub fn request(description: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self::new(ApiErrorKind::RequestException, description, start)
    }

    pub fn invalid_dataset(description: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self::new(ApiErrorKind::InvalidDatasetException, description, start)
    }

    pub fn prediction(description: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self::new(ApiErrorKind::PredictionException, description, start)
    }

    pub fn internal(description: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self::new(ApiErrorKind::InternalException, description, start)
    }

    pub fn not_implemented(description: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self::new(ApiErrorKind::NotYetImplementedException, description, start)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.kind.status();
        let body = ErrorResponse {
            error: ErrorBody {
                kind: self.kind.type_name().to_string(),
                description: self.description,
            },
            metadata: Metadata {
                start_datetime: rfc3339(self.start_datetime),
                complete_datetime: rfc3339(self.complete_datetime),
            },
        };
        (status, Json(body)).into_response()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.type_name(), self.description)
    }
}

impl std::error::Error for ApiError {}
