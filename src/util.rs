use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{ready, stream::Stream};
use pin_project_lite::pin_project;
use serde::{Deserialize, Serialize};
use shakmaty::ByColor;

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

pub trait DeduplicateStreamExt: Stream {
    fn deduplicate_by<F, T>(self, f: F) -> DeduplicatedStream<Self, F, T>
    where
        Self: Sized,
    {
        DeduplicatedStream {
            stream: self,
            f,
            latest: None,
        }
    }
}

impl<S> DeduplicateStreamExt for S where S: Stream {}

pin_project! {
    pub struct DeduplicatedStream<S, F, T> where S: Stream {
        #[pin]
        stream: S,
        latest: Option<T>,
        f: F,
    }
}

impl<S, F, T> Stream for DeduplicatedStream<S, F, T>
where
    S: Stream,
    F: FnMut(&S::Item) -> T,
    T: PartialEq,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        let mut this = self.project();

        Poll::Ready(loop {
            if let Some(item) = ready!(this.stream.as_mut().poll_next(cx)) {
                let latest = this.latest.replace((this.f)(&item));
                if latest != *this.latest {
                    break Some(item);
                }
            } else {
                break None;
            }
        })
    }
}
