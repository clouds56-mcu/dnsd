use std::{future::Future, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use futures::{future::FutureExt, Stream, StreamExt as _};

use hickory_server::proto::error::ProtoError;
use hickory_server::proto::op::NoopMessageFinalizer;
// use hickory_server::proto::udp::{DnsUdpSocket, UdpClientConnect, UdpClientStream};
use hickory_server::proto::xfer::{DnsExchange, DnsExchangeBackground, DnsExchangeConnect, DnsExchangeSend, DnsHandle, DnsRequest, DnsResponse};
use hickory_server::resolver::config::{NameServerConfig, Protocol, ResolverOpts};
use hickory_server::resolver::error::ResolveError;
use hickory_server::resolver::name_server::{ConnectionProvider, GenericConnector, RuntimeProvider, Spawn, TokioRuntimeProvider};
use hickory_server::resolver::TokioHandle;

use super::udp_stream::{StateUdpConnect, StateUdpStream};

#[derive(Clone)]
pub struct TimeWindowUdpProvider {
  inner: GenericConnector<TokioRuntimeProvider>,
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
      inner: GenericConnector::new(runtime_provider.clone()),
      runtime_provider,
      timeout: Duration::from_secs(3),
    }
  }
}

// type UdpClientStream<S> = StateUdpStream<S>;
// type UdpClientConnect<S> = StateUdpConnect<S>;

type UdpClientStream<S> = hickory_server::proto::udp::UdpClientStream<S>;
type UdpClientConnect<S> = hickory_server::proto::udp::UdpClientConnect<S>;
type ConnectionConnect<R = TokioRuntimeProvider> = DnsExchangeConnect<UdpClientConnect<<R as RuntimeProvider>::Udp>, UdpClientStream<<R as RuntimeProvider>::Udp>, <R as RuntimeProvider>::Timer>;
type ConnectionBackgroud<R = TokioRuntimeProvider> = DnsExchangeBackground<UdpClientStream<<R as RuntimeProvider>::Udp>, <R as RuntimeProvider>::Timer>;
impl ConnectionProvider for TimeWindowUdpProvider {
  type Conn = GenericConnection;
  type FutureConn = ConnectionFuture<ConnectionConnect>;
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
        ConnectionFuture {
          inner: exchange,
          spawner: self.runtime_provider.create_handle(),
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
pub struct ConnectionResponse(DnsExchangeSend);

impl Stream for ConnectionResponse {
  type Item = Result<DnsResponse, ResolveError>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
    let result = std::task::ready!(self.0.poll_next_unpin(cx).map_err(|e| e.into()));
    if let Some(Ok(ref result)) = result {
      debug!("received response: {} {:?}", result.id(), result.answers());
    }
    trace!("received response: {:?}", result);
    std::task::Poll::Ready(result)
  }
}

/// A connected DNS handle
#[derive(Clone)]
pub struct GenericConnection(DnsExchange);

impl DnsHandle for GenericConnection {
  type Response = ConnectionResponse;
  type Error = ResolveError;

  fn send<R: Into<DnsRequest> + Unpin + Send + 'static>(&self, request: R) -> Self::Response {
    let request: DnsRequest = request.into();
    debug!("sending request: {} {:?}", request.id(), request.query());
    ConnectionResponse(self.0.send(request))
  }
}

// #[derive(Clone)]
pub struct ConnectionFuture<Conn> {
  inner: Conn,
  spawner: TokioHandle,
}

impl<Conn> Future for ConnectionFuture<Conn>
where
  Conn: Future<Output=Result<(DnsExchange, ConnectionBackgroud), ProtoError>> + Unpin,
{
  type Output = Result<GenericConnection, ResolveError>;

  fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
    let this = Pin::get_mut(self);
    let (conn, bg) = std::task::ready!(this.inner.poll_unpin(cx))?;
    this.spawner.spawn_bg(bg);
    std::task::Poll::Ready(Ok(GenericConnection(conn)))
  }
}

#[test]
fn test_time_window_udp_provider() {
  let provider = TimeWindowUdpProvider::new();
  let _ = provider;
}
