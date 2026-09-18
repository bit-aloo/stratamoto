use std::collections::BinaryHeap;

use crate::time::instant::Instant;

#[derive(Default)]
pub struct Timer {
    queue: BinaryHeap<Event>,
}

impl Timer {
    pub fn push(&mut self, deadline: Instant, callback: impl FnOnce() + Send + Sync + 'static) {
        self.queue.push(Event {
            deadline,
            callback: Box::new(callback),
        });
    }

    pub fn next_time(&self) -> Option<Instant> {
        self.queue.peek().map(|e| e.deadline)
    }

    pub fn fire(&mut self, time: Instant) {
        while let Some(t) = self.next_time() {
            if t > time {
                break;
            }
            let event = self.queue.pop().unwrap();
            (event.callback)()
        }
    }
}

struct Event {
    deadline: Instant,
    callback: Box<dyn FnOnce() + Send + Sync + 'static>,
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.deadline.eq(&other.deadline)
    }
}

impl Eq for Event {}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Event {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.deadline.cmp(&self.deadline)
    }
}
