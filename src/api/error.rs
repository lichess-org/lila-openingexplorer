use std::{error::Error as StdError, fmt};

use axum::http::StatusCode;
use shakmaty::{uci::IllegalUciError, variant::VariantPosition, PositionError};

use crate::model::GameId;

#[derive(Debug)]
pub enum Error {
    PositionError(PositionError<VariantPosition>),
    IllegalUciError(IllegalUciError),
    DuplicateGame(GameId),
    RejectedImport(GameId),
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
            Error::PositionError(err) => write!(f, "bad request: {}", err),
            Error::IllegalUciError(err) => write!(f, "bad request: {}", err),
            Error::DuplicateGame(id) => write!(f, "duplicate game {}", id),
            Error::RejectedImport(id) => write!(f, "rejected import of {}", id),
        }
    }
}

impl axum::response::IntoResponse for Error {
    type Body = axum::body::Body;
    type BodyError = <Self::Body as axum::body::HttpBody>::Error;

    fn into_response(self) -> axum::http::Response<Self::Body> {
        axum::http::Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Self::Body::from(self.to_string()))
            .unwrap()
    }
}
