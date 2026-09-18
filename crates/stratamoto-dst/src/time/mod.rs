use std::{
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};

use futures::{FutureExt, future::poll_fn, select};

use crate::time::timer::Timer;

mod instant;
pub use instant::Instant;
mod timer;

pub struct TimeRuntime {
    handle: TimeHandle,
}

impl Default for TimeRuntime {
    fn default() -> Self {
        Self::new()
    }
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
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub struct TimeHandle {
    timer: Arc<Mutex<Timer>>,
    clock: ClockHandle,
}

impl TimeHandle {
    pub fn try_current() -> Option<Self> {
        crate::context::try_time_handle()
    }

    pub fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }

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

#[derive(Debug, PartialEq)]
pub struct Elapsed;

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

#[derive(Debug)]
struct Clock {
    base: std::time::Instant,
    advance: Duration,
}

#[derive(Clone)]
struct ClockHandle {
    inner: Arc<Mutex<Clock>>,
}

impl ClockHandle {
    fn new() -> Self {
        let clock = Clock {
            base: std::time::Instant::now(),
            advance: Duration::default(),
        };
        ClockHandle {
            inner: Arc::new(Mutex::new(clock)),
        }
    }

    fn set(&self, time: Instant) {
        let mut inner = self.inner.lock().unwrap();
        inner.advance = time.into_std().duration_since(inner.base);
    }

    fn now(&self) -> Instant {
        let inner = self.inner.lock().unwrap();
        Instant::from_std(inner.base + inner.advance)
    }

    fn elapsed(&self) -> Duration {
        let inner = self.inner.lock().unwrap();
        inner.advance
    }
}
