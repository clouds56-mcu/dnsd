// https://github.com/hickory-dns/hickory-dns/issues/1669

use std::{collections::{HashMap, VecDeque}, io, net::{Ipv4Addr, Ipv6Addr, SocketAddr}, sync::Arc, task::{ready, Context, Poll}};

use hickory_server::{proto::{self, udp::UdpSocket as _, TokioTime}, resolver::{name_server::{GenericConnector, RuntimeProvider, Spawn}, TokioHandle}};
use tokio::sync::{mpsc, RwLock};

pub type MemoryState = HashMap<(SocketAddr, SocketAddr), VecDeque<Packet>>;

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
    handle.spawn_bg(async move {
      while let Some(packet) = channel.recv().await {
        let mut state = state.write().await;
        let entry = match packet {
          Packet::Udp { local_addr, remote_addr, .. } => {
            debug!("receiving from {} {}", local_addr, remote_addr);
            state.entry((local_addr, remote_addr))
          }
        };
        entry.or_default().push_back(packet)
      }
      Ok(())
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
  raw: tokio::net::UdpSocket,
  sender: mpsc::Sender<Packet>,
  // receiver: mpsc::Receiver<Packet>,
  received: Arc<RwLock<MemoryState>>,
}

impl UdpSocket {
  pub async fn new(addr: SocketAddr, sender: mpsc::Sender<Packet>, received: Arc<RwLock<MemoryState>>) -> io::Result<Self> {
    Ok(Self {
      raw: tokio::net::UdpSocket::bind(addr).await?, sender, received
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

  async fn bind(addr: SocketAddr) -> io::Result<Self> {
    Ok(UdpSocket {
      raw: tokio::net::UdpSocket::bind(addr).await?,
      sender: todo!(),
      received: todo!(),
    })
  }
}

#[async_trait::async_trait]
impl proto::udp::DnsUdpSocket for UdpSocket {
  type Time = proto::TokioTime;

  fn poll_recv_from(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<(usize, SocketAddr)>> {
    let mut buf = tokio::io::ReadBuf::new(buf);
    let addr = ready!(self.raw.poll_recv_from(cx, &mut buf))?;
    let len = buf.filled().len();

    Poll::Ready(Ok((len, addr)))
  }

  fn poll_send_to(&self, cx: &mut Context<'_>, buf: &[u8], target: SocketAddr) -> Poll<io::Result<usize>> {
    self.raw.poll_send_to(cx, buf, target)
  }
}
