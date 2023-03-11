use axum::{http::StatusCode, response::Response};
use shakmaty::{san::SanError, uci::IllegalUciError, variant::VariantPosition, PositionError};
use thiserror::Error;

use crate::model::{GameId, LaxDate};

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("bad request: {0}")]
    PositionError(Box<PositionError<VariantPosition>>),
    #[error("bad request: {0}")]
    IllegalUciError(#[from] IllegalUciError),
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
}

impl From<PositionError<VariantPosition>> for Error {
    fn from(error: PositionError<VariantPosition>) -> Error {
        Error::PositionError(Box::new(error))
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            match self {
                Error::IndexerQueueFull => StatusCode::SERVICE_UNAVAILABLE,
                Error::PositionError(_)
                | Error::IllegalUciError(_)
                | Error::SanError(_)
                | Error::DuplicateGame { .. }
                | Error::RejectedRating { .. }
                | Error::RejectedDate { .. } => StatusCode::BAD_REQUEST,
            },
            self.to_string(),
        )
            .into_response()
    }
}
