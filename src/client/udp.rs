use std::{net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr}, pin::Pin, time::Duration};

use futures::Future;
use hickory_server::{proto::{iocompat::AsyncIoTokioAsStd, udp::DnsUdpSocket, TokioTime}, resolver::{name_server::RuntimeProvider, TokioHandle}};
use rand::distributions::{Distribution, Uniform};
use tokio::net::{TcpStream, UdpSocket};

use super::udp_stream::UdpCreator;


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


#[must_use = "futures do nothing unless polled"]
pub struct NextRandomUdpSocket<S> {
  name_server: SocketAddr,
  bind_address: SocketAddr,
  closure: UdpCreator<S>,
  // marker: PhantomData<S>,
}
impl<S: DnsUdpSocket> NextRandomUdpSocket<S> {
  /// Create a future with generator
  pub(crate) fn new_with_closure(name_server: &SocketAddr, func: UdpCreator<S>) -> Self {
    let bind_address = match *name_server {
      SocketAddr::V4(..) => SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0),
      SocketAddr::V6(..) => {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)), 0)
      }
    };
    Self {
      name_server: *name_server,
      bind_address,
      closure: func,
      // marker: PhantomData,
    }
  }
}
impl<S: DnsUdpSocket + Send> Future for NextRandomUdpSocket<S> {
  type Output = Result<S, std::io::Error>;

  /// polls until there is an available next random UDP port,
  /// if no port has been specified in bind_addr.
  ///
  /// if there is no port available after 10 attempts, returns NotReady
  fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
    use std::task::Poll;
    if self.bind_address.port() == 0 {
      // Per RFC 6056 Section 2.1:
      //
      //    The dynamic port range defined by IANA consists of the 49152-65535
      //    range, and is meant for the selection of ephemeral ports.
      let rand_port_range = Uniform::new_inclusive(49152_u16, u16::max_value());
      let mut rand = rand::thread_rng();

      for attempt in 0..10 {
        let port = rand_port_range.sample(&mut rand);
        let bind_addr = SocketAddr::new(self.bind_address.ip(), port);

        // TODO: allow TTL to be adjusted...
        // TODO: this immediate poll might be wrong in some cases...
        match (*self.closure)(bind_addr, self.name_server)
            .as_mut()
            .poll(cx)
        {
          Poll::Ready(Ok(socket)) => {
            debug!("created socket successfully");
            return Poll::Ready(Ok(socket));
          }
          Poll::Ready(Err(err)) => match err.kind() {
            std::io::ErrorKind::AddrInUse => {
              debug!("unable to bind port, attempt: {}: {}", attempt, err);
            }
            _ => {
              debug!("failed to bind port: {}", err);
              return Poll::Ready(Err(err));
            }
          },
          Poll::Pending => debug!("unable to bind port, attempt: {}", attempt),
        }
      }

      debug!("could not get next random port, delaying");

      // TODO: because no interest is registered anywhere, we must awake.
      cx.waker().wake_by_ref();

      // returning NotReady here, perhaps the next poll there will be some more socket available.
      Poll::Pending
    } else {
      // Use port that was specified in bind address.
      (*self.closure)(self.bind_address, self.name_server)
        .as_mut()
        .poll(cx)
    }
  }
}
