use std::{
    io, net::SocketAddr, sync::{Arc, Mutex},
};

use bytes::Bytes;

use log::trace;

use crate::{
    net::network::{Message, Network, Stat}, rand::RandomHandle, time::TimeHandle,
};

mod network;

pub struct NetworkRuntime {
    handle: NetworkHandle,
}

#[derive(Clone)]
pub struct NetworkLocalHandle {
    network: Arc<Mutex<Network>>,
    addr: SocketAddr,
    receiver: async_channel::Receiver<Message>,
}

impl NetworkRuntime {
    pub fn new(rand: RandomHandle, time: TimeHandle) -> Self {
        let handle = NetworkHandle {
            network: Arc::new(Mutex::new(Network::new(rand, time))),
        };
        NetworkRuntime { handle }
    }

    pub fn handle(&self) -> &NetworkHandle {
        &self.handle
    }
}

#[derive(Clone)]
pub struct NetworkHandle {
    network: Arc<Mutex<Network>>,
}

impl NetworkHandle {
    pub fn local_handle(&self, addr: SocketAddr) -> NetworkLocalHandle {
        let receiver = self.network.lock().unwrap().get(addr);
        NetworkLocalHandle {
            network: self.network.clone(),
            addr,
            receiver,
        }
    }

    pub fn stat(&self) -> Stat {
        self.network.lock().unwrap().stat().clone()
    }

    pub fn set_packet_loss_rate(&self, rate: f64) {
        let mut network = self.network.lock().unwrap();
        network.update_config(|cfg| cfg.packet_loss_rate = rate);
    }

    pub fn connect(&self, addr: SocketAddr) {
        let mut network = self.network.lock().unwrap();
        network.unclog(addr);
    }

    pub fn disconnect(&self, addr: SocketAddr) {
        let mut network = self.network.lock().unwrap();
        network.clog(&addr);
    }
}

impl NetworkLocalHandle {
    pub fn current() -> Self {
        crate::context::net_local_handle()
    }

    pub async fn send_to(&self, dst: SocketAddr, tag: u64, data: &[u8]) -> io::Result<()> {
        self.network.lock().unwrap().send(self.addr, dst, tag, data);
        Ok(())
    }

    pub async fn recv(&self) -> io::Result<(Bytes, u64, SocketAddr)> {
        let msg = self.receiver.recv().await.unwrap();
        trace!(
            "recv: {} <- {}, tag = {}, len = {}",
            self.addr,
            msg.from,
            msg.tag,
            msg.data.len()
        );
        Ok((msg.data, msg.tag, msg.from))
    }

    /// Take a message that has already been delivered, without waiting for one.
    pub fn try_recv(&self) -> Option<(Bytes, u64, SocketAddr)> {
        let msg = self.receiver.try_recv().ok()?;
        Some((msg.data, msg.tag, msg.from))
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
