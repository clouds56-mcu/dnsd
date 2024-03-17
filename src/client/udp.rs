use std::{net::SocketAddr, pin::Pin, time::Duration};

use futures::Future;
use hickory_server::{proto::{iocompat::AsyncIoTokioAsStd, udp::DnsUdpSocket, TokioTime}, resolver::{name_server::RuntimeProvider, TokioHandle}};
use tokio::net::{TcpStream, UdpSocket};


/// The Tokio Runtime for async execution
#[derive(Clone, Default)]
pub struct TokioRuntimeProvider(TokioHandle);
impl TokioRuntimeProvider {
  /// Create a Tokio runtime
  pub fn new() -> Self {
    Self::default()
  }
}
impl RuntimeProvider for TokioRuntimeProvider {
  type Handle = TokioHandle;
  type Timer = TokioTime;
  type Udp = UdpSocketRetransmitted;
  type Tcp = AsyncIoTokioAsStd<TcpStream>;

  fn create_handle(&self) -> Self::Handle {
    self.0.clone()
  }

  fn connect_tcp(
    &self,
    server_addr: SocketAddr,
  ) -> Pin<Box<dyn Send + Future<Output = std::io::Result<Self::Tcp>>>> {
    Box::pin(async move {
      TcpStream::connect(server_addr)
        .await
        .map(AsyncIoTokioAsStd)
    })
  }

  fn bind_udp(
    &self,
    local_addr: SocketAddr,
    server_addr: SocketAddr,
  ) -> Pin<Box<dyn Send + Future<Output = std::io::Result<Self::Udp>>>> {
    Box::pin(UdpSocketRetransmitted::bind(local_addr, server_addr))
  }
}

pub struct UdpSocketRetransmitted {
  socket: UdpSocket,
  server_addr: SocketAddr,
  timeout: Duration,
}
impl UdpSocketRetransmitted {
  pub async fn bind(local_addr: SocketAddr, server_addr: SocketAddr) -> std::io::Result<Self> {
    let socket = UdpSocket::bind(local_addr).await?;
    Ok(Self {
      socket,
      server_addr,
      timeout: Duration::from_secs(3),
    })
  }
}
#[async_trait::async_trait]
impl DnsUdpSocket for UdpSocketRetransmitted {
  type Time = TokioTime;

  fn poll_recv_from(
    &self,
    cx: &mut std::task::Context<'_> ,
    buf: &mut [u8],
  ) -> std::task::Poll<std::io::Result<(usize,SocketAddr)> > {
    let mut buf = tokio::io::ReadBuf::new(buf);
    let addr = std::task::ready!(UdpSocket::poll_recv_from(&self.socket, cx, &mut buf))?;
    let len = buf.filled().len();
    warn!("received {} bytes from {}", len, addr);
    debug!("bytes: {:?}", buf.filled());
    // std::task::Poll::Ready(Ok((len, addr)))
    std::task::Poll::Pending
  }

  fn poll_send_to(
    &self,
    cx: &mut std::task::Context<'_> ,
    buf: &[u8],
    target: SocketAddr,
  ) -> std::task::Poll<std::io::Result<usize> > {
    DnsUdpSocket::poll_send_to(&self.socket, cx, buf, target)
  }
}
