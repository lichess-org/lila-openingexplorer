use std::{
    future::Future as _,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    body::HttpBody,
    http::{HeaderMap, Response},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::{ready, stream::Stream};
use pin_project_lite::pin_project;
use serde::Serialize;
use sync_wrapper::SyncWrapper;
use tokio::{
    time,
    time::{Instant, Sleep},
};

pub struct NdJson<S>(pub S);

impl<S, T> IntoResponse for NdJson<S>
where
    S: Stream<Item = T> + Send + 'static,
    T: Serialize,
{
    type Body = NdJsonBody<S>;
    type BodyError = serde_json::Error;

    fn into_response(self) -> Response<NdJsonBody<S>> {
        Response::builder()
            .header("X-Accel-Buffering", "no")
            .header(axum::http::header::CONTENT_TYPE, "application/x-ndjson")
            .body(NdJsonBody {
                stream: SyncWrapper::new(self.0),
                keep_alive: KeepAlive::new(Duration::from_secs(8)),
            })
            .unwrap()
    }
}

pin_project! {
    pub struct NdJsonBody<S> {
        #[pin]
        stream: SyncWrapper<S>,
        #[pin]
        keep_alive: KeepAlive,
    }
}

impl<S, T> HttpBody for NdJsonBody<S>
where
    S: Stream<Item = T>,
    T: Serialize,
{
    type Data = Bytes;
    type Error = serde_json::Error;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        let this = self.project();

        let without_keepalive = this.stream.get_pin_mut().poll_next(cx).map(|item| {
            item.map(|item| {
                serde_json::to_vec(&item).map(|mut buf| {
                    buf.push(b'\n');
                    Bytes::from(buf)
                })
            })
        });

        match without_keepalive {
            Poll::Pending => {
                ready!(this.keep_alive.poll_interval(cx));
                Poll::Ready(Some(Ok(Bytes::from("\n"))))
            }
            Poll::Ready(Some(Ok(event))) => {
                this.keep_alive.reset();
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(end_or_err) => Poll::Ready(end_or_err),
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }
}

pin_project! {
    struct KeepAlive {
        interval: Duration,
        #[pin]
        sleep: Sleep,
    }
}

impl KeepAlive {
    fn new(interval: Duration) -> KeepAlive {
        KeepAlive {
            interval,
            sleep: time::sleep(interval),
        }
    }

    fn reset(self: Pin<&mut Self>) {
        let this = self.project();
        this.sleep.reset(Instant::now() + *this.interval);
    }

    fn poll_interval(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.as_mut().project();
        ready!(this.sleep.poll(cx));
        self.reset();
        Poll::Ready(())
    }
}
