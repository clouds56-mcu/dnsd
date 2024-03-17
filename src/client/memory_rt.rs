// https://github.com/hickory-dns/hickory-dns/issues/1669

use std::{collections::{HashMap, VecDeque}, io::{self, Write}, net::{Ipv4Addr, Ipv6Addr, SocketAddr}, sync::Arc, task::{Context, Poll}, time::Duration};

use hickory_server::{proto::{self, TokioTime}, resolver::{name_server::{GenericConnector, RuntimeProvider, Spawn}, TokioHandle}};
use tokio::sync::{mpsc, Mutex, RwLock};

pub struct MemoryBuffer {
  queue: Mutex<VecDeque<Packet>>,
  last_used: Mutex<tokio::time::Instant>,
}
pub type MemoryState = HashMap<SocketAddr, MemoryBuffer>;
impl MemoryBuffer {
  pub fn new() -> Self {
    Self {
      queue: Mutex::new(VecDeque::new()),
      last_used: Mutex::new(tokio::time::Instant::now()),
    }
  }
}

#[derive(Clone)]
pub struct MemoryRuntime {
  handle: TokioHandle,
  sender: mpsc::Sender<Packet>,
  // receiver: mpsc::Receiver<Packet>,
  received: Arc<RwLock<MemoryState>>,
}

impl MemoryRuntime {
  pub fn new() -> (GenericConnector<Self>, mpsc::Sender<Packet>, mpsc::Receiver<Packet>) {
    let handle = TokioHandle::default();
    let (sender, out_receiver) = mpsc::channel(1024);
    let (out_sender, receiver) = mpsc::channel(1024);
    let received = Arc::new(RwLock::new(HashMap::new()));
    Self::sync(handle.clone(), receiver, received.clone());
    let rt = Self {
      handle, sender, received,
    };
    (GenericConnector::new(rt), out_sender, out_receiver)
  }

  pub fn sync(mut handle: TokioHandle, mut channel: mpsc::Receiver<Packet>, state: Arc<RwLock<MemoryState>>) {
    let state_to_clean = state.clone();
    handle.spawn_bg(async move {
      info!("backgroud sync running");
      while let Some(packet) = channel.recv().await {
        trace!("memory::sync: package arrived");
        let mut state = state.write().await;
        let entry = match packet {
          Packet::Udp { local_addr, remote_addr, .. } => {
            trace!("memory::sync: receiving from {} {}", local_addr, remote_addr);
            state.entry(local_addr)
          }
        };
        entry.or_insert_with(MemoryBuffer::new).queue.lock().await.push_back(packet);
      }
      Ok(())
    });
    handle.spawn_bg(async move {
      loop {
        let mut to_clean = vec![];
        tokio::time::sleep(Duration::from_secs(1)).await;
        let state = state_to_clean.read().await;
        for (addr, buf) in state.iter() {
          if buf.last_used.lock().await.elapsed() > Duration::from_secs(10) {
            trace!("memory::sync: cleaning up {:?}", addr);
            buf.queue.lock().await.clear();
            to_clean.push(*addr);
          }
        }
        drop(state); // no upgradable read
        if let Ok(mut state) = state_to_clean.try_write() {
          for addr in to_clean {
            state.remove(&addr);
          }
        }
      }
    })
  }
}

impl RuntimeProvider for MemoryRuntime {
  type Handle = TokioHandle;
  type Timer = TokioTime;
  type Tcp = proto::iocompat::AsyncIoTokioAsStd<tokio::net::TcpStream>;
  type Udp = UdpSocket;

  fn create_handle(&self) -> Self::Handle {
    self.handle.clone()
  }

  fn connect_tcp(&self, _server_addr: SocketAddr) -> std::pin::Pin<Box<dyn Send + futures::prelude::Future<Output = io::Result<Self::Tcp>>>> {
    unimplemented!("tcp not support now")
  }

  fn bind_udp(&self, local_addr: SocketAddr, _server_addr: SocketAddr) -> std::pin::Pin<Box<dyn Send + futures::prelude::Future<Output = io::Result<Self::Udp>>>> {
    Box::pin(UdpSocket::new(local_addr, self.sender.clone(), self.received.clone()))
  }
}

#[derive(Debug, Clone)]
pub enum Packet {
  Udp {
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
    buffer: Vec<u8>,
  }
}


pub struct UdpSocket {
  addr: SocketAddr,
  sender: mpsc::Sender<Packet>,
  // receiver: mpsc::Receiver<Packet>,
  received: Arc<RwLock<MemoryState>>,
}

impl UdpSocket {
  pub async fn new(addr: SocketAddr, sender: mpsc::Sender<Packet>, received: Arc<RwLock<MemoryState>>) -> io::Result<Self> {
    Ok(Self {
      addr, sender, received,
    })
  }
}

#[async_trait::async_trait]
impl proto::udp::UdpSocket for UdpSocket {
  async fn connect(addr: SocketAddr) -> io::Result<Self> {
    let bind_addr: SocketAddr = match addr {
      SocketAddr::V4(_addr) => (Ipv4Addr::UNSPECIFIED, 0).into(),
      SocketAddr::V6(_addr) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    Self::connect_with_bind(addr, bind_addr).await
  }

  async fn connect_with_bind(_addr: SocketAddr, bind_addr: SocketAddr) -> io::Result<Self> {
    let socket = Self::bind(bind_addr).await?;
    // TODO: research connect more, it appears to break UDP receiving tests, etc...
    // socket.connect(addr).await?;
    Ok(socket)
  }

  async fn bind(_addr: SocketAddr) -> io::Result<Self> {
    todo!()
  }
}

#[async_trait::async_trait]
impl proto::udp::DnsUdpSocket for UdpSocket {
  type Time = proto::TokioTime;

  fn poll_recv_from(&self, _cx: &mut Context<'_>, _buff: &mut [u8]) -> Poll<io::Result<(usize, SocketAddr)>> {
    // let mut buf = tokio::io::ReadBuf::new(buf);
    // let addr = ready!(self.raw.poll_recv_from(cx, &mut buf))?;
    // let len = buf.filled().len();

    // Poll::Ready(Ok((len, addr)))
    todo!()
  }

  async fn recv_from(&self, mut buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
    loop {
      let state = self.received.read().await;
      if let Some(entry) = state.get(&self.addr) {
        if let Some(item) = entry.queue.lock().await.pop_front() {
          match item {
            Packet::Udp { local_addr: _, remote_addr, buffer } => {
              buf.write_all(&buffer)?;
              return Ok((buffer.len(), remote_addr))
            },
          }
        }
      }
    }
  }

  fn poll_send_to(&self, _cx: &mut Context<'_>, _buf: &[u8], _target: SocketAddr) -> Poll<io::Result<usize>> {
    // self.raw.poll_send_to(cx, buf, target)
    todo!();
  }

  async fn send_to(&self, buf: &[u8], target: SocketAddr) -> io::Result<usize> {
    trace!("UdpSocket::send_to {target} len={}", buf.len());
    if let Some(state) = self.received.read().await.get(&self.addr) {
      *state.last_used.lock().await = tokio::time::Instant::now();
    }
    self.sender.send(Packet::Udp { local_addr: self.addr, remote_addr: target, buffer: buf.to_owned() }).await.map_err(|_| io::ErrorKind::UnexpectedEof)?;
    Ok(buf.len())
  }
}
