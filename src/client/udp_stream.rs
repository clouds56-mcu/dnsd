use std::{net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use futures::{Future, FutureExt, Stream, StreamExt};
use hickory_server::proto::{error::ProtoError, op::NoopMessageFinalizer, udp::{DnsUdpSocket, UdpClientConnect, UdpClientStream}, xfer::{DnsRequest, DnsRequestSender, DnsResponseStream}};
use tokio::net::UdpSocket;

pub(crate) type UdpCreator<S> = Arc<dyn Fn(SocketAddr, SocketAddr) -> Pin<Box<dyn Future<Output = Result<S, std::io::Error>> + Send>> + Send + Sync>;

pub struct StateUdpStream<S: Send = UdpSocket> {
  inner: UdpClientStream<S>
}
impl<S: DnsUdpSocket + Send> StateUdpStream<S> {
  pub fn with_creator(
    server_addr: SocketAddr,
    message_finalizer: Option<Arc<NoopMessageFinalizer>>,
    timeout: Duration,
    creator: UdpCreator<S>,
  ) -> StateUdpConnect<S> {
    let inner = UdpClientStream::with_creator(server_addr, message_finalizer, timeout, creator);
    StateUdpConnect { inner }
  }
}
impl<S: DnsUdpSocket + Send + 'static> DnsRequestSender for StateUdpStream<S> {
  fn send_message(&mut self, message: DnsRequest) -> DnsResponseStream {
    info!("sending message: {} {:?}", message.id(), message.query());
    self.inner.send_message(message)
  }

  fn shutdown(&mut self) {
    self.inner.shutdown()
  }

  fn is_shutdown(&self) -> bool {
    self.inner.is_shutdown()
  }
}
impl<S: DnsUdpSocket + Send> Stream for StateUdpStream<S> {
  type Item = Result<(), ProtoError>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
    self.inner.poll_next_unpin(cx)
  }
}


pub struct StateUdpConnect<S: Send = UdpSocket> {
  inner: UdpClientConnect<S>,
}

impl<S: Send + Unpin> Future for StateUdpConnect<S> {
  type Output = Result<StateUdpStream<S>, ProtoError>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
    trace!("polling StateUdpConnect");
    self.inner.poll_unpin(cx).map_ok(|inner| StateUdpStream { inner })
  }
}
