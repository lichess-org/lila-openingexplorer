use axum::http::StatusCode;
use std::error::Error as StdError;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    IndexerQueueFull,
    IndexerRequestError(reqwest::Error),
    IndexerStreamError(io::Error),
}

impl StdError for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::IndexerQueueFull => f.write_str("indexer queue full"),
            Error::IndexerRequestError(err) => write!(f, "indexer request error: {}", err),
            Error::IndexerStreamError(err) => write!(f, "indexer stream error: {}", err),
        }
    }
}

impl axum::response::IntoResponse for Error {
    type Body = axum::body::Body;
    type BodyError = <Self::Body as axum::body::HttpBody>::Error;

    fn into_response(self) -> axum::http::Response<Self::Body> {
        axum::http::Response::builder()
            .status(match self {
                Error::IndexerQueueFull
                | Error::IndexerRequestError(_)
                | Error::IndexerStreamError(_) => StatusCode::SERVICE_UNAVAILABLE,
            })
            .body(Self::Body::from(self.to_string()))
            .unwrap()
    }
}
