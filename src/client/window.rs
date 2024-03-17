

use std::{future::Future, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use futures::{future::FutureExt, Stream, StreamExt as _};
use hickory_server::{proto::{op::NoopMessageFinalizer, udp::{UdpClientConnect, UdpClientStream}, xfer::{DnsExchange, DnsExchangeConnect, DnsExchangeSend, DnsHandle, DnsRequest, DnsResponse}, TokioTime}, resolver::{config::{NameServerConfig, Protocol, ResolverOpts}, error::ResolveError, name_server::{ConnectionProvider, RuntimeProvider, Spawn, TokioConnectionProvider, TokioRuntimeProvider}, TokioHandle}};
use tokio::{net::UdpSocket, time::Sleep};

#[derive(Clone)]
pub struct TimeWindowUdpProvider {
  inner: TokioConnectionProvider,
  runtime_provider: TokioRuntimeProvider,
  timeout: Duration,
}

impl std::fmt::Debug for TimeWindowUdpProvider {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("TimeWindowUdpProvider").finish()
  }
}

impl TimeWindowUdpProvider {
  pub fn new() -> Self {
    let runtime_provider = TokioRuntimeProvider::new();
    Self {
      inner: TokioConnectionProvider::new(runtime_provider.clone()),
      runtime_provider,
      timeout: Duration::from_secs(3),
    }
  }
}

impl ConnectionProvider for TimeWindowUdpProvider {
  type Conn = GenericConnection;
  type FutureConn = TimeWindowUdpConnection;
  type RuntimeProvider = TokioRuntimeProvider;

  fn new_connection(&self, config: &NameServerConfig, options: &ResolverOpts) -> Self::FutureConn {
    match config.protocol {
      Protocol::Udp => {
        let provider_handle = self.runtime_provider.clone();
        let closure = move |local_addr: SocketAddr, server_addr: SocketAddr| {
            provider_handle.bind_udp(local_addr, server_addr)
        };
        let stream = UdpClientStream::with_creator(
            config.socket_addr,
            None::<Arc<NoopMessageFinalizer>>,
            options.timeout,
            Arc::new(closure),
        );
        let exchange = DnsExchange::connect(stream);
        TimeWindowUdpConnection {
          inner: exchange,
          spawner: self.runtime_provider.create_handle(),
          timeout: self.timeout,
        }
      },
      _ => {
        let _ = self.inner.new_connection(config, options);
        unimplemented!()
      }
    }
  }
}

/// A stream of response to a DNS request.
#[must_use = "steam do nothing unless polled"]
pub struct ConnectionResponse {
  stream: Option<DnsExchangeSend>,
  received: Option<Result<DnsResponse, ResolveError>>,
  timeout: Pin<Box<Sleep>>,
}

impl Stream for ConnectionResponse {
  type Item = Result<DnsResponse, ResolveError>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
    let this = Pin::get_mut(self);
    if let Some(stream) = this.stream.as_mut() {
      match std::task::ready!(stream.poll_next_unpin(cx)) {
        Some(Ok(result)) => {
          debug!("received response: {:?}", result);
          this.received = Some(Ok(result));
        },
        Some(Err(e)) => {
          if this.received.is_none() || this.received.as_ref().unwrap().is_err() {
            this.received = Some(Err(ResolveError::from(e)));
          }
        },
        None => this.stream = None,
      }
    }
    std::task::ready!(this.timeout.poll_unpin(cx));
    std::task::Poll::Ready(this.received.take())
  }
}

/// A connected DNS handle
#[derive(Clone)]
pub struct GenericConnection(DnsExchange, Duration);

impl DnsHandle for GenericConnection {
  type Response = ConnectionResponse;
  type Error = ResolveError;

  fn send<R: Into<DnsRequest> + Unpin + Send + 'static>(&self, request: R) -> Self::Response {
    let request: DnsRequest = request.into();
    debug!("sending request: {} {:?}", request.id(), request.query());
    ConnectionResponse {
      stream: Some(self.0.send(request)),
      received: None,
      timeout: Box::pin(tokio::time::sleep(self.1)),
    }
  }
}

// #[derive(Clone)]
pub struct TimeWindowUdpConnection {
  inner: DnsExchangeConnect<UdpClientConnect<UdpSocket>, UdpClientStream<UdpSocket>, TokioTime>,
  spawner: TokioHandle,
  timeout: Duration,
}

impl Future for TimeWindowUdpConnection {
  type Output = Result<GenericConnection, ResolveError>;

  fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
    let this = Pin::get_mut(self);
    let (conn, bg) = std::task::ready!(this.inner.poll_unpin(cx))?;
    this.spawner.spawn_bg(bg);
    std::task::Poll::Ready(Ok(GenericConnection(conn, this.timeout)))
  }
}

#[test]
fn test_time_window_udp_provider() {
  let provider = TimeWindowUdpProvider::new();
  let _ = provider;
}
