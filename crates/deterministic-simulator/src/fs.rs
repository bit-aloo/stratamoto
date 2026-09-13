use std::{
    collections::HashMap,
    io::{self, Error},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

use log::trace;

use crate::{rand::RandomHandle, time::TimeHandle};

pub struct File {
    inode: Arc<INode>,
    can_write: bool,
}

struct INode {
    path: PathBuf,
    data: RwLock<Vec<u8>>,
}

#[derive(Clone)]
pub struct FileSystemLocalHandle {
    addr: SocketAddr,
    fs: Arc<Mutex<HashMap<PathBuf, Arc<INode>>>>,
}

#[derive(Clone)]
pub struct FileSystemHandle {
    handles: Arc<Mutex<HashMap<SocketAddr, FileSystemLocalHandle>>>,
    rand: RandomHandle,
    time: TimeHandle,
}

pub struct FileSystemRuntime {
    handle: FileSystemHandle,
}

impl FileSystemRuntime {
    pub(crate) fn new(rand: RandomHandle, time: TimeHandle) -> Self {
        let handle = FileSystemHandle {
            handles: Arc::new(Mutex::new(HashMap::new())),
            rand,
            time,
        };
        FileSystemRuntime { handle }
    }

    pub fn handle(&self) -> &FileSystemHandle {
        &self.handle
    }
}

impl FileSystemHandle {
    pub fn local_handle(&self, addr: SocketAddr) -> FileSystemLocalHandle {
        let mut handles = self.handles.lock().unwrap();
        handles
            .entry(addr)
            .or_insert_with(|| FileSystemLocalHandle::new(addr))
            .clone()
    }
}

impl FileSystemLocalHandle {
    fn new(addr: SocketAddr) -> Self {
        trace!("fs: new at {}", addr);
        FileSystemLocalHandle {
            addr,
            fs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn current() -> Self {
        crate::context::fs_local_handle()
    }

    pub async fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        let path = path.as_ref();
        trace!("fs({}): open at {:?}", self.addr, path);
        let fs = self.fs.lock().unwrap();
        let inode = fs
            .get(path)
            .ok_or(Error::new(
                io::ErrorKind::NotFound,
                format!("file not found: {path:?}"),
            ))?
            .clone();
        Ok(File {
            inode,
            can_write: false,
        })
    }

    pub async fn create(&self, path: impl AsRef<Path>) -> io::Result<File> {
        let path = path.as_ref();
        trace!("fs({}): create at {:?}", self.addr, path);
        let mut fs = self.fs.lock().unwrap();
        let inode = fs
            .entry(path.into())
            .or_insert_with(|| Arc::new(INode::new(path)))
            .clone();
        Ok(File {
            inode,
            can_write: true,
        })
    }
}

impl INode {
    fn new(path: &Path) -> Self {
        INode {
            path: path.into(),
            data: RwLock::new(Vec::new()),
        }
    }
}

impl File {
    pub async fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        trace!(
            "file({:?}): read_at: offset={}, len={}",
            self.inode.path,
            offset,
            buf.len()
        );

        let data = self.inode.data.read().unwrap();
        let end = data.len().min(offset as usize + buf.len());
        let len = end - offset as usize;
        buf[..len].copy_from_slice(&data[offset as usize..end]);
        Ok(len)
    }

    pub async fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        trace!(
            "file({:?}): write_all_at: offset={}, len={}",
            self.inode.path,
            offset,
            buf.len()
        );

        if !self.can_write {
            return Err(Error::new(
                io::ErrorKind::PermissionDenied,
                "the file is read only",
            ));
        }
        let mut data = self.inode.data.write().unwrap();
        let end = data.len().min(offset as usize + buf.len());
        let len = end - offset as usize;
        data[offset as usize..end].copy_from_slice(&buf[..len]);
        if len < buf.len() {
            data.extend_from_slice(&buf[len..]);
        }

        Ok(())
    }

    pub async fn set_len(&self, size: u64) -> io::Result<()> {
        trace!("file({:?}): set_len={}", self.inode.path, size,);
        let mut data = self.inode.data.write().unwrap();
        data.resize(size as usize, 0);
        Ok(())
    }
}
