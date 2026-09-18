use std::{cell::RefCell, net::SocketAddr};

use crate::Handle;

thread_local! {
    static CONTEXT: RefCell<Option<Handle>> = const { RefCell::new(None) };
    static ADDR: RefCell<Option<SocketAddr>> = const { RefCell::new(None) };
}

pub(crate) fn current() -> Option<Handle> {
    CONTEXT.with(|ctx| ctx.borrow().clone())
}

pub(crate) fn time_handle() -> crate::time::TimeHandle {
    CONTEXT.with(|ctx| ctx.borrow().as_ref().unwrap().time.clone())
}

pub(crate) fn task_local_handle() -> crate::task::TaskLocalHandle {
    let addr = ADDR.with(|addr| addr.borrow().unwrap());
    CONTEXT.with(|ctx| ctx.borrow().as_ref().unwrap().task.local_handle(addr))
}

pub fn try_time_handle() -> Option<crate::time::TimeHandle> {
    CONTEXT.with(|ctx| ctx.borrow().as_ref().map(|h| h.time.clone()))
}

pub(crate) fn net_local_handle() -> crate::net::NetworkLocalHandle {
    let addr = ADDR.with(|addr| addr.borrow().unwrap());
    CONTEXT.with(|ctx| ctx.borrow().as_ref().unwrap().net.local_handle(addr))
}

pub(crate) fn fs_local_handle() -> crate::fs::FileSystemLocalHandle {
    let addr = ADDR.with(|addr| addr.borrow().unwrap());
    CONTEXT.with(|ctx| ctx.borrow().as_ref().unwrap().fs.local_handle(addr))
}

pub(crate) fn enter(new: Handle) -> EnterGuard {
    CONTEXT.with(|ctx| {
        let old = ctx.borrow_mut().replace(new);
        EnterGuard(old)
    })
}

pub(crate) fn rand_handle() -> crate::rand::RandomHandle {
    CONTEXT.with(|ctx| ctx.borrow().as_ref().unwrap().rand.clone())
}

pub(crate) struct EnterGuard(Option<Handle>);

impl Drop for EnterGuard {
    fn drop(&mut self) {
        CONTEXT.with(|ctx| *ctx.borrow_mut() = self.0.take())
    }
}

pub(crate) struct TaskEnterGuard(Option<SocketAddr>);

impl Drop for TaskEnterGuard {
    fn drop(&mut self) {
        ADDR.with(|ctx| *ctx.borrow_mut() = self.0.take());
    }
}

pub(crate) fn enter_task(new: SocketAddr) -> TaskEnterGuard {
    ADDR.with(|ctx| {
        let old = ctx.borrow_mut().replace(new);
        TaskEnterGuard(old)
    })
}
