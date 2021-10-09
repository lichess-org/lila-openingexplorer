use std::error::Error as StdError;
use axum::http::StatusCode;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    IndexerTooBusy,
}

impl StdError for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::IndexerTooBusy => "indexer too busy",
        })
    }
}

impl axum::response::IntoResponse for Error {
    type Body = axum::body::Body;
    type BodyError = <Self::Body as axum::body::HttpBody>::Error;

    fn into_response(self) -> axum::http::Response<Self::Body> {
        axum::http::Response::builder()
            .status(match self {
                Error::IndexerTooBusy => StatusCode::SERVICE_UNAVAILABLE,
            })
            .body(Self::Body::from(self.to_string()))
            .unwrap()
    }
}
