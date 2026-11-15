use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use tokio::sync::{Notify, watch};

use crate::player::UserId;

/// Progress of the indexing of a player, as seen by the requests waiting
/// for it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    /// Position in the queue (1 for the next player), 0 while indexing.
    pub queue_position: u64,
    /// Incremented after each committed batch of new games.
    pub generation: u64,
    /// Indexing is complete.
    pub done: bool,
}

/// The progress of the indexing of a player.
#[derive(Clone)]
pub struct Ticket {
    rx: watch::Receiver<Progress>,
}

impl Ticket {
    /// A ticket for a player that does not need indexing.
    pub fn completed() -> Ticket {
        let (_, rx) = watch::channel(Progress {
            queue_position: 0,
            generation: 0,
            done: true,
        });
        Ticket { rx }
    }

    pub fn progress(&self) -> Progress {
        self.rx.borrow().clone()
    }

    /// Waits for the next change of the progress, returning it. Returns the
    /// last progress (as done) if the queue dropped the ticket.
    pub async fn changed(&mut self) -> Progress {
        match self.rx.changed().await {
            Ok(()) => self.rx.borrow_and_update().clone(),
            Err(_) => Progress {
                done: true,
                ..*self.rx.borrow()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFull;

struct Entry {
    tx: Arc<watch::Sender<Progress>>,
    /// Has an index already (an incremental run), queued before first runs.
    known: bool,
}

#[derive(Default)]
struct State {
    entries: HashMap<UserId, Entry>,
    waiting: VecDeque<UserId>,
    /// Players being indexed, rebuilt or deleted, who must not be indexed
    /// again meanwhile.
    busy: HashSet<UserId>,
    /// Players waiting for exclusive access (see `Queue::reserve`), who are
    /// not indexed meanwhile either.
    reserving: HashMap<UserId, usize>,
    /// Players being indexed.
    running: usize,
}

impl State {
    /// Updates the queue positions of the waiting players from `from` (the
    /// players before it did not move).
    fn update_positions(&self, from: usize) {
        for (i, user) in self.waiting.iter().enumerate().skip(from) {
            if let Some(entry) = self.entries.get(user) {
                entry.tx.send_if_modified(|p| {
                    let position = i as u64 + 1;
                    let changed = p.queue_position != position;
                    p.queue_position = position;
                    changed
                });
            }
        }
    }

    /// Removes the waiting players that nobody waits for any more.
    fn remove_abandoned(&mut self) {
        let State {
            entries, waiting, ..
        } = self;
        let mut first = None;
        let mut i = 0;
        waiting.retain(|user| {
            let abandoned = entries
                .get(user)
                .is_none_or(|entry| entry.tx.receiver_count() == 0);
            if abandoned {
                entries.remove(user);
                first.get_or_insert(i);
            }
            i += 1;
            !abandoned
        });
        if let Some(first) = first {
            self.update_positions(first);
        }
    }
}

pub struct Queue {
    state: Mutex<State>,
    notify: Notify,
    /// Notified whenever a player stops being busy.
    released: Notify,
    capacity: usize,
    known_first: bool,
}

/// Counts of the queue, for `/metrics`.
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub waiting: usize,
    pub running: usize,
    pub busy: usize,
}

impl Queue {
    pub fn new(capacity: usize, known_first: bool) -> Queue {
        Queue {
            state: Mutex::new(State::default()),
            notify: Notify::new(),
            released: Notify::new(),
            capacity,
            known_first,
        }
    }

    pub fn stats(&self) -> QueueStats {
        let state = self.state.lock().expect("queue");
        QueueStats {
            waiting: state.waiting.len(),
            running: state.running,
            busy: state.busy.len(),
        }
    }

    /// The ticket of a player that is queued or being indexed.
    pub fn watch(&self, user: &UserId) -> Option<Ticket> {
        let state = self.state.lock().expect("queue");
        state.entries.get(user).map(|entry| Ticket {
            rx: entry.tx.subscribe(),
        })
    }

    /// Queues a player, or joins the ticket of the player if already
    /// queued. With `known_first`, players that are `known` (have an index
    /// already) are queued before the first waiting player that is not.
    pub fn submit(&self, user: UserId, known: bool) -> Result<Ticket, QueueFull> {
        let mut state = self.state.lock().expect("queue");
        if let Some(entry) = state.entries.get(&user) {
            return Ok(Ticket {
                rx: entry.tx.subscribe(),
            });
        }
        if state.waiting.len() >= self.capacity {
            // While all indexers are busy, abandoned players are not removed
            // by `acquire`.
            state.remove_abandoned();
            if state.waiting.len() >= self.capacity {
                return Err(QueueFull);
            }
        }
        let (tx, rx) = watch::channel(Progress {
            queue_position: state.waiting.len() as u64 + 1,
            generation: 0,
            done: false,
        });
        state.entries.insert(
            user.clone(),
            Entry {
                tx: Arc::new(tx),
                known,
            },
        );
        let position = if known && self.known_first {
            state
                .waiting
                .iter()
                .position(|other| state.entries.get(other).is_some_and(|e| !e.known))
                .unwrap_or(state.waiting.len())
        } else {
            state.waiting.len()
        };
        state.waiting.insert(position, user);
        state.update_positions(position);
        drop(state);
        self.notify.notify_one();
        Ok(Ticket { rx })
    }

    fn try_acquire(self: &Arc<Self>) -> Option<QueueItem> {
        let mut state = self.state.lock().expect("queue");
        let mut i = 0;
        let mut acquired = None;
        // The first index whose player moved.
        let mut moved = None;
        while i < state.waiting.len() {
            let user = &state.waiting[i];
            let entry = state.entries.get(user).expect("entry of waiting player");
            if entry.tx.receiver_count() == 0 {
                // Nobody waits for it any more.
                let user = state.waiting.remove(i).expect("waiting player");
                state.entries.remove(&user);
                moved.get_or_insert(i);
                continue;
            }
            if state.busy.contains(user) || state.reserving.contains_key(user) {
                i += 1;
                continue;
            }
            let user = state.waiting.remove(i).expect("waiting player");
            moved.get_or_insert(i);
            let entry = state.entries.get(&user).expect("entry");
            entry.tx.send_modify(|p| p.queue_position = 0);
            let tx = Arc::clone(&entry.tx);
            state.busy.insert(user.clone());
            state.running += 1;
            acquired = Some(QueueItem {
                user,
                tx,
                queue: Arc::clone(self),
                finished: false,
            });
            break;
        }
        if let Some(moved) = moved {
            state.update_positions(moved);
        }
        acquired
    }

    /// Waits until no index run or rebuild of the player is in progress,
    /// and keeps new ones from starting until the guard is dropped (for
    /// exclusive access, like deleting the player). Index runs of the
    /// player do not start while this waits.
    pub async fn reserve(self: &Arc<Self>, user: &UserId) -> BusyGuard {
        *self
            .state
            .lock()
            .expect("queue")
            .reserving
            .entry(user.clone())
            .or_default() += 1;
        loop {
            let released = self.released.notified();
            {
                let mut state = self.state.lock().expect("queue");
                if !state.busy.contains(user) {
                    state.busy.insert(user.clone());
                    if let Some(n) = state.reserving.get_mut(user) {
                        *n -= 1;
                        if *n == 0 {
                            state.reserving.remove(user);
                        }
                    }
                    return BusyGuard {
                        user: user.clone(),
                        queue: Arc::clone(self),
                    };
                }
            }
            released.await;
        }
    }

    /// Wakes the indexers and reservations after a player stopped being
    /// busy.
    fn wake(&self) {
        self.notify.notify_one();
        self.released.notify_waiters();
    }

    /// Waits for the next player to index.
    pub async fn acquire(self: &Arc<Self>) -> QueueItem {
        loop {
            let notified = self.notify.notified();
            if let Some(item) = self.try_acquire() {
                return item;
            }
            notified.await;
        }
    }
}

/// A player taken from the queue for indexing. Dropping it completes the
/// ticket.
pub struct QueueItem {
    user: UserId,
    tx: Arc<watch::Sender<Progress>>,
    queue: Arc<Queue>,
    finished: bool,
}

impl QueueItem {
    pub fn user(&self) -> &UserId {
        &self.user
    }

    /// A handle to notify the requests of committed batches.
    pub fn progress(&self) -> ProgressHandle {
        ProgressHandle {
            tx: Arc::clone(&self.tx),
        }
    }

    /// Whether no request waits for the player any more.
    pub fn abandoned(&self) -> bool {
        self.tx.receiver_count() == 0
    }

    /// Puts the player back at the end of the queue, to continue later.
    pub fn requeue(mut self) {
        self.finished = true;
        let mut state = self.queue.state.lock().expect("queue");
        state.busy.remove(&self.user);
        state.running -= 1;
        if let Some(entry) = state.entries.get_mut(&self.user) {
            entry.known = true;
            state.waiting.push_back(self.user.clone());
            state.update_positions(state.waiting.len() - 1);
        }
        drop(state);
        self.queue.wake();
    }

    /// Completes the ticket, but keeps the player busy (not indexed again)
    /// until the returned guard is dropped.
    pub fn finish_busy(mut self) -> BusyGuard {
        self.finished = true;
        self.complete(false);
        BusyGuard {
            user: self.user.clone(),
            queue: Arc::clone(&self.queue),
        }
    }

    fn complete(&self, release: bool) {
        let mut state = self.queue.state.lock().expect("queue");
        state.entries.remove(&self.user);
        state.running -= 1;
        if release {
            state.busy.remove(&self.user);
        }
        self.tx.send_modify(|p| {
            p.done = true;
            p.queue_position = 0;
        });
        drop(state);
        if release {
            self.queue.wake();
        }
    }
}

impl Drop for QueueItem {
    fn drop(&mut self) {
        if !self.finished {
            self.complete(true);
        }
    }
}

/// Keeps a player busy, so that it is not indexed (for rebuilds).
pub struct BusyGuard {
    user: UserId,
    queue: Arc<Queue>,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.queue
            .state
            .lock()
            .expect("queue")
            .busy
            .remove(&self.user);
        self.queue.wake();
    }
}

#[derive(Clone)]
pub struct ProgressHandle {
    tx: Arc<watch::Sender<Progress>>,
}

impl ProgressHandle {
    /// Notifies that a batch was committed.
    pub fn committed(&self) {
        self.tx.send_modify(|p| p.generation += 1);
    }

    /// A handle that is not part of a queue, and its ticket.
    #[cfg(test)]
    pub fn detached() -> (ProgressHandle, Ticket) {
        let (tx, rx) = watch::channel(Progress::default());
        (ProgressHandle { tx: Arc::new(tx) }, Ticket { rx })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        pin::{Pin, pin},
        task::{Context, Poll, Waker},
    };

    use super::*;

    fn user(name: &str) -> UserId {
        UserId::from_name(name).unwrap()
    }

    /// Polls a future once, without waiting.
    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn test_queue() {
        let queue = Arc::new(Queue::new(2, false));
        let a = queue.submit(user("a"), false).unwrap();
        let b = queue.submit(user("b"), false).unwrap();
        assert_eq!(queue.submit(user("c"), false).err(), Some(QueueFull));
        let a2 = queue.submit(user("a"), false).unwrap();
        assert_eq!(a.progress().queue_position, 1);
        assert_eq!(b.progress().queue_position, 2);

        let item = queue.try_acquire().unwrap();
        assert_eq!(item.user(), &user("a"));
        assert_eq!(a2.progress().queue_position, 0);
        assert_eq!(b.progress().queue_position, 1);
        assert_eq!(queue.stats().running, 1);
        item.progress().committed();
        assert_eq!(a.progress().generation, 1);

        // Requeued behind b.
        item.requeue();
        assert_eq!(a.progress().queue_position, 2);
        assert_eq!(b.progress().queue_position, 1);
        assert_eq!(queue.stats().running, 0);

        // Abandoned tickets are dropped.
        drop(b);
        let item = queue.try_acquire().unwrap();
        assert_eq!(item.user(), &user("a"));
        let busy = item.finish_busy();
        assert!(a.progress().done);
        assert!(queue.watch(&user("a")).is_none());
        assert_eq!(queue.stats().running, 0);

        // Busy players wait.
        let a3 = queue.submit(user("a"), false).unwrap();
        assert!(queue.try_acquire().is_none());
        drop(busy);
        let item = queue.try_acquire().unwrap();
        assert_eq!(item.user(), &user("a"));
        drop(item);
        assert!(a3.progress().done);
        assert_eq!(queue.stats().busy, 0);
        assert_eq!(queue.stats().running, 0);
    }

    /// A full queue makes room by removing players nobody waits for.
    #[test]
    fn test_full_removes_abandoned() {
        let queue = Arc::new(Queue::new(2, false));
        let a = queue.submit(user("a"), false).unwrap();
        let b = queue.submit(user("b"), false).unwrap();
        drop(a);
        let c = queue.submit(user("c"), false).unwrap();
        assert_eq!(b.progress().queue_position, 1);
        assert_eq!(c.progress().queue_position, 2);
        assert!(queue.watch(&user("a")).is_none());
        assert_eq!(queue.submit(user("d"), false).err(), Some(QueueFull));
    }

    #[test]
    fn test_reserve() {
        let queue = Arc::new(Queue::new(10, false));
        let a = user("a");
        let _ticket = queue.submit(a.clone(), false).unwrap();
        let item = queue.try_acquire().unwrap();

        // Waits for the run to end.
        let mut reserve = pin!(queue.reserve(&a));
        assert!(poll_once(reserve.as_mut()).is_pending());

        // A requeued run does not start again while the reservation waits.
        item.requeue();
        let Poll::Ready(guard) = poll_once(reserve.as_mut()) else {
            panic!("reserved");
        };
        assert!(queue.try_acquire().is_none());

        // It continues after the reservation.
        drop(guard);
        let item = queue.try_acquire().unwrap();
        assert_eq!(item.user(), &a);
        drop(item);
        assert_eq!(queue.stats().busy, 0);
    }

    #[test]
    fn test_known_first() {
        let queue = Arc::new(Queue::new(10, true));
        let a = queue.submit(user("a"), false).unwrap();
        let b = queue.submit(user("b"), true).unwrap();
        let c = queue.submit(user("c"), false).unwrap();
        let d = queue.submit(user("d"), true).unwrap();
        let order: Vec<u64> = [&a, &b, &c, &d]
            .iter()
            .map(|t| t.progress().queue_position)
            .collect();
        assert_eq!(order, [3, 1, 4, 2]);
    }
}
