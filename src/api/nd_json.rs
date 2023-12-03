use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    body::Body,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::{ready, stream::Stream};
use pin_project_lite::pin_project;
use serde::Serialize;
use sync_wrapper::SyncWrapper;
use tokio::{
    time,
    time::{Interval, MissedTickBehavior},
};

pub struct NdJson<S>(pub S);

impl<S, T> IntoResponse for NdJson<S>
where
    S: Stream<Item = T> + Send + 'static,
    T: Serialize,
{
    fn into_response(self) -> Response {
        let mut keep_alive = time::interval(Duration::from_secs(8));
        keep_alive.set_missed_tick_behavior(MissedTickBehavior::Delay);

        Response::builder()
            .header("X-Accel-Buffering", "no")
            .header(axum::http::header::CONTENT_TYPE, "application/x-ndjson")
            .body(Body::from_stream(NdJsonStream {
                item_stream: SyncWrapper::new(self.0),
                keep_alive,
            }))
            .unwrap()
    }
}

pin_project! {
    pub struct NdJsonStream<S> {
        #[pin]
        item_stream: SyncWrapper<S>,
        keep_alive: Interval,
    }
}

impl<S, T> Stream for NdJsonStream<S>
where
    S: Stream<Item = T>,
    T: Serialize,
{
    type Item = Result<Bytes, serde_json::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        let without_keepalive = this.item_stream.get_pin_mut().poll_next(cx).map(|item| {
            item.map(|item| {
                serde_json::to_vec(&item).map(|mut buf| {
                    buf.push(b'\n');
                    Bytes::from(buf)
                })
            })
        });

        match without_keepalive {
            Poll::Pending => {
                ready!(this.keep_alive.poll_tick(cx));
                Poll::Ready(Some(Ok(Bytes::from("\n"))))
            }
            Poll::Ready(Some(Ok(event))) => {
                this.keep_alive.reset();
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(end_or_err) => Poll::Ready(end_or_err),
        }
    }
}
