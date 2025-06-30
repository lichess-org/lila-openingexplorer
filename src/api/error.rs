use std::sync::Arc;

use axum::{http::StatusCode, response::Response};
use shakmaty::{PositionError, san::SanError, uci::IllegalUciMoveError, variant::VariantPosition};
use thiserror::Error;

use crate::model::{GameId, LaxDate};

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("bad request: {0}")]
    PositionError(Box<PositionError<VariantPosition>>),
    #[error("bad request: {0}")]
    IllegalUciMoveError(#[from] IllegalUciMoveError),
    #[error("bad request: {0}")]
    SanError(#[from] SanError),
    #[error("duplicate game {id}")]
    DuplicateGame { id: GameId },
    #[error("rejected import of {id} due to average rating {rating}")]
    RejectedRating { id: GameId, rating: u16 },
    #[error("rejected import of {id} due to date {date}")]
    RejectedDate { id: GameId, date: LaxDate },
    #[error("indexer queue full")]
    IndexerQueueFull,
    #[error("duplicate opening position")]
    DuplicateOpening,
    #[error("bad request: {0}")]
    CsvError(Arc<csv::Error>),
    #[error("internal request failed: {0}")]
    ReqwestError(Arc<reqwest::Error>),
}

impl From<PositionError<VariantPosition>> for Error {
    fn from(error: PositionError<VariantPosition>) -> Error {
        Error::PositionError(Box::new(error))
    }
}

impl From<csv::Error> for Error {
    fn from(error: csv::Error) -> Error {
        Error::CsvError(Arc::new(error))
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Error {
        Error::ReqwestError(Arc::new(error))
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            match self {
                Error::IndexerQueueFull => StatusCode::SERVICE_UNAVAILABLE,
                Error::PositionError(_)
                | Error::IllegalUciMoveError(_)
                | Error::SanError(_)
                | Error::DuplicateGame { .. }
                | Error::RejectedRating { .. }
                | Error::RejectedDate { .. }
                | Error::CsvError(_)
                | Error::DuplicateOpening => StatusCode::BAD_REQUEST,
                Error::ReqwestError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            },
            self.to_string(),
        )
            .into_response()
    }
}
