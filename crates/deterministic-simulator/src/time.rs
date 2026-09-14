use std::{
    collections::BinaryHeap,
    sync::{Arc, Mutex},
    task::Poll,
    time::{Duration, Instant},
};

use futures::{FutureExt, future::poll_fn, select};

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

#[derive(Default)]
struct Timer {
    queue: BinaryHeap<Event>,
}

impl Timer {
    fn push(&mut self, deadline: Instant, callback: impl FnOnce() + Send + Sync + 'static) {
        self.queue.push(Event {
            deadline,
            callback: Box::new(callback),
        });
    }

    fn next_time(&self) -> Option<Instant> {
        self.queue.peek().map(|e| e.deadline)
    }

    fn fire(&mut self, time: Instant) {
        while let Some(t) = self.next_time() {
            if t > time {
                break;
            }
            let event = self.queue.pop().unwrap();
            (event.callback)();
        }
    }
}

#[derive(Debug)]
struct Clock {
    /// Time basis for which mock time is derived.
    base: Instant,
    /// The amount of mock time which has elapsed.
    advance: Duration,
}

#[derive(Clone)]
struct ClockHandle {
    inner: Arc<Mutex<Clock>>,
}

impl ClockHandle {
    fn new() -> Self {
        let clock = Clock {
            base: Instant::now(),
            advance: Duration::default(),
        };
        ClockHandle {
            inner: Arc::new(Mutex::new(clock)),
        }
    }

    fn set(&self, time: Instant) {
        let mut inner = self.inner.lock().unwrap();
        inner.advance = time.duration_since(inner.base);
    }

    fn now(&self) -> Instant {
        let inner = self.inner.lock().unwrap();
        inner.base + inner.advance
    }
}

#[derive(Debug, PartialEq)]
pub struct Elapsed;

pub fn now() -> Instant {
    let handle = crate::context::time_handle();
    handle.now()
}

pub fn sleep(duration: Duration) -> impl Future<Output = ()> {
    let handle = crate::context::time_handle();
    handle.sleep(duration)
}

pub fn sleep_until(deadline: Instant) -> impl Future<Output = ()> {
    let handle = crate::context::time_handle();
    handle.sleep_until(deadline)
}

pub fn timeout<T: Future>(
    duration: Duration,
    future: T,
) -> impl Future<Output = Result<T::Output, Elapsed>> {
    let handle = crate::context::time_handle();
    handle.timeout(duration, future)
}

#[derive(Clone)]
pub struct TimeHandle {
    timer: Arc<Mutex<Timer>>,
    clock: ClockHandle,
}

impl TimeHandle {
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    pub fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + use<> {
        self.sleep_until(self.clock.now() + duration)
    }

    pub fn sleep_until(&self, deadline: Instant) -> impl Future<Output = ()> + use<> {
        let handle = self.clone();
        poll_fn(move |cx| {
            if handle.clock.now() >= deadline {
                return Poll::Ready(());
            }

            let waker = cx.waker().clone();
            handle.add_timer(deadline, || waker.wake());
            Poll::Pending
        })
    }

    pub fn timeout<T: Future>(
        &self,
        duration: Duration,
        future: T,
    ) -> impl Future<Output = Result<T::Output, Elapsed>> + use<T> {
        let timeout = self.sleep(duration);
        async move {
            select! {
                res = future.fuse() => Ok(res),
                _ = timeout.fuse() => Err(Elapsed)
            }
        }
    }

    pub fn add_timer(&self, deadline: Instant, callback: impl FnOnce() + Send + Sync + 'static) {
        let mut timer = self.timer.lock().unwrap();
        timer.push(deadline, callback);
    }
}

pub(crate) struct TimeRuntime {
    handle: TimeHandle,
}

impl TimeRuntime {
    pub fn new() -> Self {
        let handle = TimeHandle {
            timer: Arc::new(Mutex::new(Timer::default())),
            clock: ClockHandle::new(),
        };
        TimeRuntime { handle }
    }

    pub fn handle(&self) -> &TimeHandle {
        &self.handle
    }

    pub fn advance(&self) -> bool {
        let mut timer = self.handle.timer.lock().unwrap();
        if let Some(time) = timer.next_time() {
            timer.fire(time);
            self.handle.clock.set(time);
            return true;
        }
        false
    }

    pub fn now(&self) -> Instant {
        self.handle.now()
    }
}
