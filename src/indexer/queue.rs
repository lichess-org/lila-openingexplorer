use std::{
    collections::{
        hash_map::{Entry, HashMap},
        VecDeque,
    },
    hash::Hash,
    sync::{Arc, Mutex},
};

use tokio::sync::{watch, Notify};

pub struct Queue<T> {
    state: Mutex<QueueState<T>>,
    notify: Notify,
}

impl<T: Eq + Hash + Clone> Queue<T> {
    pub fn with_capacity(capacity: usize) -> Queue<T> {
        Queue {
            state: Mutex::new(QueueState::with_capacity(capacity)),
            notify: Notify::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().len()
    }

    pub fn preceding_tickets(&self, ticket: &Ticket) -> u64 {
        ticket
            .number
            .saturating_sub(self.state.lock().unwrap().acquired_number)
    }

    pub fn submit(&self, task: T) -> Result<Ticket, QueueFull<T>> {
        let result = self.state.lock().unwrap().submit(task);
        if result.is_ok() {
            self.notify.notify_one();
        }
        result
    }

    pub async fn acquire(self: Arc<Self>) -> QueueItem<T> {
        let task = loop {
            if let Some(task) = self.state.lock().unwrap().acquire() {
                break task;
            }
            self.notify.notified().await;
        };

        QueueItem { task, queue: self }
    }
}

pub struct QueueFull<T>(pub T);

struct QueueState<T> {
    indexing: HashMap<T, QueuePosition>,
    queue: VecDeque<T>,
    next_number: u64,
    acquired_number: u64,
}

impl<T: Eq + Hash + Clone> QueueState<T> {
    fn with_capacity(capacity: usize) -> QueueState<T> {
        QueueState {
            indexing: HashMap::with_capacity(capacity),
            queue: VecDeque::with_capacity(capacity),
            next_number: 0,
            acquired_number: 0,
        }
    }

    fn len(&self) -> usize {
        self.indexing.len()
    }

    fn submit(&mut self, task: T) -> Result<Ticket, QueueFull<T>> {
        let entry = match self.indexing.entry(task) {
            Entry::Occupied(entry) => return Ok(entry.get().ticket()),
            Entry::Vacant(entry) => entry,
        };

        if self.queue.len() >= self.queue.capacity() {
            return Err(QueueFull(entry.into_key()));
        }

        self.queue.push_back(entry.key().clone());

        let queue_position = entry.insert(QueuePosition::with_number(self.next_number));
        self.next_number += 1;
        Ok(queue_position.ticket())
    }

    fn acquire(&mut self) -> Option<T> {
        while let Some(task) = self.queue.pop_front() {
            let entry = match self.indexing.entry(task) {
                Entry::Occupied(entry) => entry,
                Entry::Vacant(_) => continue, // Should not be possible
            };

            self.acquired_number = entry.get().number;

            if entry.get().tx.is_closed() {
                entry.remove();
            } else {
                return Some(entry.key().clone());
            }
        }
        None
    }

    fn complete(&mut self, task: &T) {
        self.indexing.remove(task);
    }
}

struct QueuePosition {
    tx: watch::Sender<()>,
    number: u64,
}

impl QueuePosition {
    fn with_number(number: u64) -> QueuePosition {
        let (tx, _) = watch::channel(());
        QueuePosition { tx, number }
    }

    fn ticket(&self) -> Ticket {
        Ticket {
            rx: self.tx.subscribe(),
            number: self.number,
        }
    }
}

pub struct Ticket {
    rx: watch::Receiver<()>,
    number: u64,
}

impl Ticket {
    pub async fn completed(&mut self) {
        let _ = self.rx.changed().await;
    }
}

pub struct QueueItem<T: Eq + Hash + Clone> {
    task: T,
    queue: Arc<Queue<T>>,
}

impl<T: Eq + Hash + Clone> QueueItem<T> {
    pub fn task(&self) -> &T {
        &self.task
    }
}

impl<T: Eq + Hash + Clone> Drop for QueueItem<T> {
    fn drop(&mut self) {
        self.queue.state.lock().unwrap().complete(&self.task);
    }
}
