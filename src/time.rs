use std::time::Duration;

#[derive(Clone)]
pub struct TimeHandle;

impl TimeHandle {
    pub fn new() -> Self {
        TimeHandle
    }

    pub fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + use<> {
        tokio::time::sleep(duration)
    }
}
