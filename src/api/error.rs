use std::{error::Error as StdError, fmt};

use axum::{body, http::StatusCode, response::Response};
use shakmaty::{san::SanError, uci::IllegalUciError, variant::VariantPosition, PositionError};

use crate::model::GameId;

#[derive(Debug)]
pub enum Error {
    PositionError(PositionError<VariantPosition>),
    IllegalUciError(IllegalUciError),
    SanError(SanError),
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

impl From<SanError> for Error {
    fn from(err: SanError) -> Error {
        Error::SanError(err)
    }
}

impl StdError for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PositionError(err) => write!(f, "bad request: {}", err),
            Error::IllegalUciError(err) => write!(f, "bad request: {}", err),
            Error::SanError(err) => write!(f, "bad request: {}", err),
            Error::DuplicateGame(id) => write!(f, "duplicate game {}", id),
            Error::RejectedImport(id) => write!(f, "rejected import of {}", id),
        }
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> Response {
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(body::boxed(body::Full::from(self.to_string())))
            .unwrap()
    }
}
