use std::io;

mod executor;
mod net;
mod rand;
mod time;

pub struct Runtime {
    rand: rand::RandomHandle,
    time: time::TimeHandle,
    executor: executor::Executor,
    net: net::NetworkRuntime,
}

impl Runtime {
    pub fn new() -> io::Result<Self> {
        Self::new_with_seed(0)
    }

    pub fn new_with_seed(seed: u64) -> io::Result<Self> {
        let rand = rand::RandomHandle::new_with_seed(seed);
        let time = time::TimeHandle::new();
        let executor = executor::Executor::new()?;
        let net = net::NetworkRuntime::new(rand.clone(), time.clone(), executor.handle());
        Ok(Runtime {
            rand,
            time,
            executor,
            net,
        })
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.executor.block_on(future)
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::Runtime;

    #[test]
    fn connect() {
        env_logger::init();

        let rt = Runtime::new().unwrap();
        let addr1 = "0.0.0.1:1".parse().unwrap();
        let addr2 = "0.0.0.2:1".parse().unwrap();
        let host1 = rt.net.handle(addr1);
        let host2 = rt.net.handle(addr2);

        rt.block_on(async move {
            host1.send_to(addr2, 1, &[1]).await.unwrap();
            let mut buf = vec![0; 0x10];
            let (len, tag, from) = host2.recv_from(&mut buf).await.unwrap();
            assert_eq!(len, 1);
            assert_eq!(tag, 1);
            assert_eq!(from, addr1);
        });
    }
}