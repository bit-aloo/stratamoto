use std::{
    collections::HashMap,
    net::SocketAddr,
    ops::Deref,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::Instant,
};

use async_task::{Runnable, Task};

use crate::time::{TimeHandle, TimeRuntime};

#[derive(Debug)]
struct TaskInfo {
    addr: SocketAddr,
    killed: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct TaskLocalHandle {
    sender: mpsc::Sender<(Runnable, Arc<TaskInfo>)>,
    info: Arc<TaskInfo>,
}

impl TaskLocalHandle {
    pub fn spawn<F>(&self, future: F) -> Task<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let sender = self.sender.clone();
        let info = self.info.clone();
        let (runnable, task) = unsafe {
            async_task::spawn_unchecked(future, move |runnable| {
                sender.send((runnable, info.clone())).unwrap()
            })
        };
        runnable.schedule();
        task
    }
}

#[derive(Clone)]
pub struct TaskHandle {
    sender: mpsc::Sender<(Runnable, Arc<TaskInfo>)>,
    info: Arc<Mutex<HashMap<SocketAddr, Arc<TaskInfo>>>>,
}

impl TaskHandle {
    pub fn kill(&self, addr: SocketAddr) {
        let mut info = self.info.lock().unwrap();
        let info = info.remove(&addr).unwrap();
        info.killed.store(true, Ordering::SeqCst);
    }

    pub fn local_handle(&self, addr: SocketAddr) -> TaskLocalHandle {
        let mut info = self.info.lock().unwrap();
        let info = info
            .entry(addr)
            .or_insert_with(|| {
                Arc::new(TaskInfo {
                    addr,
                    killed: AtomicBool::new(false),
                })
            })
            .clone();
        TaskLocalHandle {
            sender: self.sender.clone(),
            info,
        }
    }
}

pub struct Executor {
    queue: mpsc::Receiver<(Runnable, Arc<TaskInfo>)>,
    handle: TaskHandle,
    time: TimeRuntime,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    pub fn new() -> Self {
        let (sender, queue) = mpsc::channel();
        Executor {
            queue,
            handle: TaskHandle {
                sender,
                info: Arc::new(Mutex::new(HashMap::new())),
            },
            time: TimeRuntime::new(),
        }
    }

    pub fn handle(&self) -> &TaskHandle {
        &self.handle
    }

    pub fn time_handle(&self) -> &TimeHandle {
        self.time.handle()
    }

    pub fn now(&self) -> Instant {
        self.time.now()
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let sender = self.handle.sender.clone();
        let info = Arc::new(TaskInfo {
            addr: "0.0.0.0:0".parse().unwrap(),
            killed: AtomicBool::new(false),
        });

        let (runnable, mut task) = unsafe {
            async_task::spawn_unchecked(future, move |runnable| {
                sender.send((runnable, info.clone())).unwrap();
            })
        };
        runnable.schedule();

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        loop {
            self.run_all_ready();
            if let Poll::Ready(value) = Pin::new(&mut task).poll(&mut cx) {
                return value;
            }
            let going = self.time.advance();
            assert!(going, "no events, the task will block forever");
        }
    }

    fn run_all_ready(&self) {
        while let Ok((runnable, info)) = self.queue.try_recv() {
            if info.killed.load(Ordering::SeqCst) {
                continue;
            }
            let _guard = crate::context::enter_task(info.addr);
            runnable.run();
        }
    }
}

impl Deref for Executor {
    type Target = TaskHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

pub fn spawn<F>(future: F) -> Task<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handle = crate::context::task_local_handle();
    handle.spawn(future)
}
