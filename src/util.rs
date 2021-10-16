use axum::{
    body::HttpBody,
    http::{HeaderMap, Response},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::stream::Stream;
use pin_project_lite::pin_project;
use serde::{Deserialize, Serialize};
use shakmaty::ByColor;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use sync_wrapper::SyncWrapper;

#[derive(Serialize, Deserialize)]
#[serde(remote = "ByColor")]
pub struct ByColorDef<T> {
    white: T,
    black: T,
}

pub trait NevermindExt: Sized {
    fn nevermind(self, _msg: &str) {}
}

impl<T, E> NevermindExt for Result<T, E> {}

pub struct NdJson<S> {
    stream: S,
}

impl<S> NdJson<S> {
    pub fn new(stream: S) -> NdJson<S> {
        NdJson { stream }
    }
}

impl<S, T, E> IntoResponse for NdJson<S>
where
    S: Stream<Item = Result<T, E>> + Send + 'static,
    T: Serialize,
    E: Into<axum::BoxError>,
{
    type Body = NdJsonBody<S>;
    type BodyError = E;

    fn into_response(self) -> Response<NdJsonBody<S>> {
        Response::builder()
            .header(axum::http::header::CONTENT_TYPE, "application/x-ndjson")
            .body(NdJsonBody {
                stream: SyncWrapper::new(self.stream),
            })
            .unwrap()
    }
}

pin_project! {
    pub struct NdJsonBody<S> {
        #[pin]
        stream: SyncWrapper<S>,
    }
}

impl<S, T, E> HttpBody for NdJsonBody<S>
where
    S: Stream<Item = Result<T, E>>,
    T: Serialize,
{
    type Data = Bytes;
    type Error = E;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        self.project()
            .stream
            .get_pin_mut()
            .poll_next(cx)
            .map_ok(|item| {
                let mut buf = serde_json::to_vec(&item).expect("serialize ndjson item");
                buf.push(b'\n');
                Bytes::from(buf)
            })
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }
}
