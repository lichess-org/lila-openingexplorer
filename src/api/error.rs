use axum::{body, http::StatusCode, response::Response};
use shakmaty::{san::SanError, uci::IllegalUciError, variant::VariantPosition, PositionError};
use thiserror::Error;

use crate::model::GameId;

#[derive(Error, Debug)]
pub enum Error {
    #[error("bad request: {0}")]
    PositionError(#[from] PositionError<VariantPosition>),
    #[error("bad request: {0}")]
    IllegalUciError(#[from] IllegalUciError),
    #[error("bad request: {0}")]
    SanError(#[from] SanError),
    #[error("duplicate game {0}")]
    DuplicateGame(GameId),
    #[error("rejected import of {0}")]
    RejectedImport(GameId),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> Response {
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(body::boxed(body::Full::from(self.to_string())))
            .unwrap()
    }
}
