use std::{
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use log::trace;

use crate::{
    executor::Spawner,
    net::network::{Config, Message, Network},
    rand::RandomHandle,
    time::TimeHandle,
};

mod network;

pub struct NetworkRuntime {
    network: Arc<Mutex<Network>>,
}

#[derive(Clone)]
pub struct NetworkHandle {
    network: Arc<Mutex<Network>>,
    addr: SocketAddr,
    receiver: async_channel::Receiver<Message>,
}

impl NetworkRuntime {
    pub fn new(rand: RandomHandle, time: TimeHandle, spawner: Spawner) -> Self {
        let config = Config::default();
        let network = Network::new(rand, time, spawner, config);
        NetworkRuntime {
            network: Arc::new(Mutex::new(network)),
        }
    }

    pub fn handle(&self, addr: SocketAddr) -> NetworkHandle {
        let receiver = self.network.lock().unwrap().insert(addr);
        NetworkHandle {
            network: self.network.clone(),
            addr,
            receiver,
        }
    }
}

impl NetworkHandle {
    pub async fn send_to(&self, dst: SocketAddr, tag: u64, data: &[u8]) -> io::Result<()> {
        self.network.lock().unwrap().send(self.addr, dst, tag, data);
        Ok(())
    }

    pub async fn recv_from(&self, data: &mut [u8]) -> io::Result<(usize, u64, SocketAddr)> {
        let msg = self.receiver.recv().await.unwrap();
        let len = data.len().min(msg.data.len());
        data[..len].copy_from_slice(&msg.data[..len]);
        trace!(
            "recv: {} <- {}, tag = {}, len = {}",
            self.addr, msg.from, msg.tag, len
        );
        Ok((len, msg.tag, msg.from))
    }
}
