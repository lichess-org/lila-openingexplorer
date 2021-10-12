use axum::http::StatusCode;
use shakmaty::{uci::IllegalUciError, variant::VariantPosition, PositionError};
use std::error::Error as StdError;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    IndexerQueueFull,
    IndexerRequestError(reqwest::Error),
    IndexerStreamError(io::Error),
    IndexerGameError { err: io::Error, line: String },
    PositionError(PositionError<VariantPosition>),
    IllegalUciError(IllegalUciError),
}

impl From<PositionError<VariantPosition>> for Error {
    fn from(err: PositionError<VariantPosition>) -> Error {
        Error::PositionError(err)
    }
}

impl From<IllegalUciError> for Error {
    fn from(err: IllegalUciError) -> Error {
        Error::IllegalUciError(err)
    }
}

impl StdError for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::IndexerQueueFull => f.write_str("indexer queue full"),
            Error::IndexerRequestError(err) => write!(f, "indexer request error: {}", err),
            Error::IndexerStreamError(err) => write!(f, "indexer stream error: {}", err),
            Error::IndexerGameError { err, line } => {
                write!(f, "indexer game error: {}: {}", err, line)
            }
            Error::PositionError(err) => write!(f, "bad request: {}", err),
            Error::IllegalUciError(err) => write!(f, "bad request: {}", err),
        }
    }
}

impl axum::response::IntoResponse for Error {
    type Body = axum::body::Body;
    type BodyError = <Self::Body as axum::body::HttpBody>::Error;

    fn into_response(self) -> axum::http::Response<Self::Body> {
        axum::http::Response::builder()
            .status(match self {
                Error::IndexerRequestError(ref err) => {
                    err.status().unwrap_or(StatusCode::SERVICE_UNAVAILABLE)
                }
                Error::IndexerQueueFull
                | Error::IndexerStreamError(_)
                | Error::IndexerGameError { .. } => StatusCode::SERVICE_UNAVAILABLE,
                Error::PositionError(_) | Error::IllegalUciError(_) => StatusCode::BAD_REQUEST,
            })
            .body(Self::Body::from(self.to_string()))
            .unwrap()
    }
}
