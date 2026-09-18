use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    ops::Range,
    time::Duration,
};

use async_channel::{Receiver, Sender};
use bytes::Bytes;
use log::trace;
use rand::Rng;

use crate::{rand::RandomHandle, time::TimeHandle};

pub(crate) struct Network {
    rand: RandomHandle,
    time: TimeHandle,
    config: Config,
    stat: Stat,
    endpoints: HashMap<SocketAddr, (Sender<Message>, Receiver<Message>)>,
    clogged: HashSet<SocketAddr>,
}

#[derive(Debug)]
pub struct Config {
    pub packet_loss_rate: f64,
    pub send_latency: Range<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            packet_loss_rate: 0.0,
            send_latency: Duration::from_millis(1)..Duration::from_millis(10),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Stat {
    pub msg_count: u64,
}

pub struct Message {
    pub tag: u64,
    pub data: Bytes,
    pub from: SocketAddr,
}

impl Network {
    pub fn new(rand: RandomHandle, time: TimeHandle) -> Self {
        Self {
            rand,
            time,
            config: Config::default(),
            stat: Stat::default(),
            endpoints: HashMap::new(),
            clogged: HashSet::new(),
        }
    }

    pub fn update_config(&mut self, f: impl FnOnce(&mut Config)) {
        f(&mut self.config)
    }

    pub fn stat(&self) -> &Stat {
        &self.stat
    }

    pub fn get(&mut self, target: SocketAddr) -> async_channel::Receiver<Message> {
        if let Some((_, recver)) = self.endpoints.get(&target) {
            return recver.clone();
        }
        let (sender, receiver) = async_channel::unbounded();
        self.endpoints.insert(target, (sender, receiver.clone()));
        receiver
    }

    #[allow(
        dead_code,
        reason = "fault injection the network offers, not yet reachable from a handle"
    )]
    pub fn remove(&mut self, target: &SocketAddr) {
        self.endpoints.remove(target);
        self.clogged.remove(target);
    }

    #[allow(
        dead_code,
        reason = "fault injection the network offers, not yet reachable from a handle"
    )]
    pub fn clog(&mut self, target: &SocketAddr) {
        self.clogged.insert(*target);
    }

    #[allow(
        dead_code,
        reason = "fault injection the network offers, not yet reachable from a handle"
    )]
    pub fn unclog(&mut self, target: SocketAddr) {
        self.clogged.insert(target);
    }

    pub fn send(&mut self, src: SocketAddr, dst: SocketAddr, tag: u64, data: &[u8]) {
        trace!("send: {} -> {}, tag={}, len={}", src, dst, tag, data.len());
        assert!(self.endpoints.contains_key(&src));
        if !self.endpoints.contains_key(&dst)
            || self.clogged.contains(&src)
            || self.clogged.contains(&dst)
            || self.rand.should_fault(self.config.packet_loss_rate)
        {
            trace!("drop");
            return;
        }

        let sender = self.endpoints[&dst].0.clone();
        let msg = Message {
            tag,
            data: Bytes::copy_from_slice(data),
            from: src,
        };
        trace!("delay: {:?}", self.config.send_latency);
        let latency = self.rand.gen_range(self.config.send_latency.clone());
        self.time.add_timer(self.time.now() + latency, move || {
            let _ = sender.try_send(msg);
        });
    }
}
