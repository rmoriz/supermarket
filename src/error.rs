use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::locales::t;

/// The API error envelope. Field order is part of the wire contract: `render_404`
/// emits messages before the code, every other call site emits the code first.
#[derive(Debug)]
pub enum ApiError {
    NotFound,
    InvalidData(Vec<String>),
    NegativeParameter { start: String, items: String },
    AuthenticationFailed(Vec<String>),
    Unauthorized(Vec<String>),
    Forbidden(Vec<String>),
    Conflict(Vec<String>),
}

impl ApiError {
    pub fn not_found() -> Self {
        Self::NotFound
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::InvalidData(_) | Self::NegativeParameter { .. } => StatusCode::BAD_REQUEST,
            Self::AuthenticationFailed(_) | Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => t_static("api.error_codes.not_found"),
            Self::InvalidData(_) | Self::NegativeParameter { .. } => {
                t_static("api.error_codes.invalid_data")
            }
            Self::AuthenticationFailed(_) => t_static("api.error_codes.authentication_failed"),
            Self::Unauthorized(_) => t_static("api.error_codes.unauthorized"),
            Self::Forbidden(_) => t_static("api.error_codes.forbidden"),
            Self::Conflict(_) => t_static("api.error_codes.conflict"),
        }
    }

    pub fn messages(&self) -> Vec<String> {
        match self {
            Self::NotFound => vec![t("api.error_messages.not_found", &[])],
            Self::InvalidData(messages)
            | Self::AuthenticationFailed(messages)
            | Self::Unauthorized(messages)
            | Self::Forbidden(messages)
            | Self::Conflict(messages) => messages.clone(),
            Self::NegativeParameter { start, items } => vec![t(
                "api.error_messages.negative_parameter",
                &[("start", start), ("items", items)],
            )],
        }
    }

    fn body(&self) -> String {
        let messages = self.messages();
        if matches!(self, Self::NotFound) {
            json!({ "error_messages": messages, "error_code": self.code() }).to_string()
        } else {
            json!({ "error_code": self.code(), "error_messages": messages }).to_string()
        }
    }
}

fn t_static(key: &str) -> &'static str {
    crate::locales::lookup(key).unwrap_or_else(|| panic!("missing locale key: {key}"))
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status(),
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            Body::from(self.body()),
        )
            .into_response()
    }
}

/// Renders like `ApplicationController#not_found!`: `{}` for JSON clients, an HTML
/// page for browsers.
pub fn not_found_html() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Body::from(crate::views::error_page(StatusCode::NOT_FOUND)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_of(err: ApiError) -> (StatusCode, String) {
        let response = err.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn not_found_body_is_byte_exact() {
        let (status, body) = body_of(ApiError::NotFound).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            body,
            r#"{"error_messages":["Resource does not exist."],"error_code":"NOT_FOUND"}"#
        );
    }

    #[tokio::test]
    async fn other_errors_put_the_code_first() {
        let (status, body) = body_of(ApiError::Forbidden(vec!["no".into()])).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body,
            r#"{"error_code":"FORBIDDEN","error_messages":["no"]}"#
        );
    }

    #[tokio::test]
    async fn negative_parameters_interpolate() {
        let (_, body) = body_of(ApiError::NegativeParameter {
            start: "-1".into(),
            items: "10".into(),
        })
        .await;
        assert_eq!(
            body,
            r#"{"error_code":"INVALID_DATA","error_messages":["Start and item parameters must be positive numbers but they are start: -1 and items: 10."]}"#
        );
    }
}
