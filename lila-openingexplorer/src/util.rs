use std::{
    cmp::min,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{ready, stream::Stream};
use partial_sort::partial_sort;
use pin_project_lite::pin_project;
use serde::{Deserialize, Serialize};
use shakmaty::{variant::VariantPosition, ByColor, Position};
use tokio::{sync::Semaphore, task};

#[derive(Serialize, Deserialize)]
#[serde(remote = "ByColor")]
pub struct ByColorDef<T> {
    black: T,
    white: T,
}

pub fn ply(pos: &VariantPosition) -> u32 {
    (u32::from(pos.fullmoves()) - 1)
        .saturating_mul(2)
        .saturating_add(pos.turn().fold_wb(0, 1))
}

pub fn sort_by_key_and_truncate<T, K, F>(vec: &mut Vec<T>, num: usize, mut f: F)
where
    F: FnMut(&T) -> K,
    K: Ord,
{
    let num = min(num, vec.len());
    partial_sort(vec, num, |a, b| f(a).lt(&f(b)));
    vec.truncate(num);
}

pub trait DedupStreamExt: Stream {
    fn dedup_by_key<F, T>(self, f: F) -> Dedup<Self, F, T>
    where
        Self: Sized,
        F: FnMut(&Self::Item) -> T,
        T: PartialEq,
    {
        Dedup {
            stream: self,
            f,
            latest: None,
        }
    }
}

impl<S> DedupStreamExt for S where S: Stream {}

pin_project! {
    pub struct Dedup<S, F, T> where S: Stream {
        #[pin]
        stream: S,
        latest: Option<T>,
        f: F,
    }
}

impl<S, F, T> Stream for Dedup<S, F, T>
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

pub fn midpoint(a: u16, b: u16) -> u16 {
    ((u32::from(a) + u32::from(b)) / 2) as u16
}

pub async fn spawn_blocking<F, R>(semaphore: &Semaphore, f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let _permit = semaphore.acquire().await.expect("semaphore not closed");
    task::spawn_blocking(f).await.expect("blocking task")
}
