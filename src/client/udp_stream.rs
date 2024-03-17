use std::{borrow::Borrow, marker::PhantomData, net::SocketAddr, pin::Pin, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};

use futures::{Future, FutureExt, Stream, StreamExt};

use hickory_server::proto::Time;
use hickory_server::proto::{error::ProtoError, op::{MessageFinalizer, NoopMessageFinalizer}, udp::{DnsUdpSocket, UdpClientConnect, UdpClientStream}, xfer::{DnsRequest, DnsRequestSender, DnsResponse, DnsResponseStream, SerialMessage}};
use tokio::net::UdpSocket;

use crate::client::udp::NextRandomUdpSocket;

pub(crate) type UdpCreator<S> = Arc<dyn Fn(SocketAddr, SocketAddr) -> Pin<Box<dyn Future<Output = Result<S, std::io::Error>> + Send>> + Send + Sync>;

const MAX_RECEIVE_BUFFER_SIZE: usize = 4096;

/// creates random query_id, each socket is unique, no need for global uniqueness
fn random_query_id() -> u16 {
  use rand::distributions::{Distribution, Standard};
  let mut rand = rand::thread_rng();

  Standard.sample(&mut rand)
}

pub struct StateUdpStream<S: Send = UdpSocket, MF=NoopMessageFinalizer> {
  // inner: UdpClientStream<S>,
  window: Duration,

  name_server: SocketAddr,
  timeout: Duration,
  is_shutdown: bool,
  signer: Option<Arc<MF>>,
  creator: UdpCreator<S>,
  // marker: PhantomData<S>,
}
impl<S: DnsUdpSocket + Send> StateUdpStream<S> {
  pub fn with_creator(
    server_addr: SocketAddr,
    message_finalizer: Option<Arc<NoopMessageFinalizer>>,
    timeout: Duration,
    creator: UdpCreator<S>,
  ) -> StateUdpConnect<S> {
    let inner = UdpClientStream::with_creator(server_addr, message_finalizer, timeout, creator);
    StateUdpConnect { inner, window: timeout }
  }
}
impl<S: DnsUdpSocket + Send + 'static, MF: MessageFinalizer> DnsRequestSender for StateUdpStream<S, MF> {
  fn send_message(&mut self, mut message: DnsRequest) -> DnsResponseStream {
    info!("sending message: {} {:?}", message.id(), message.query());

    if self.is_shutdown {
      panic!("can not send messages after stream is shutdown")
    }

    // associated the ID for this request, b/c this connection is unique to socket port, the ID
    //   does not need to be globally unique
    message.set_id(random_query_id());

    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
      Ok(now) => now.as_secs(),
      Err(_) => return ProtoError::from("Current time is before the Unix epoch.").into(),
    };

    // TODO: truncates u64 to u32, error on overflow?
    let now = now as u32;

    let mut verifier = None;
    if let Some(ref signer) = self.signer {
      if signer.should_finalize_message(&message) {
        match message.finalize::<MF>(signer.borrow(), now) {
          Ok(answer_verifier) => verifier = answer_verifier,
          Err(e) => {
            debug!("could not sign message: {}", e);
            return e.into();
          }
        }
      }
    }

    // Get an appropriate read buffer size.
    let recv_buf_size = MAX_RECEIVE_BUFFER_SIZE.min(message.max_payload() as usize);

    let bytes = match message.to_vec() {
      Ok(bytes) => bytes,
      Err(err) => {
        return err.into();
      }
    };

    let message_id = message.id();
    let message = SerialMessage::new(bytes, self.name_server);

    debug!("final message: {}", message.to_message().expect("bizarre we just made this message"));
    let creator = self.creator.clone();
    let addr = message.addr();

    S::Time::timeout::<Pin<Box<dyn Future<Output = Result<DnsResponse, ProtoError>> + Send>>>(
      self.timeout,
      Box::pin(async move {
        let socket: S = NextRandomUdpSocket::new_with_closure(&addr, creator).await?;
        // send_serial_message_inner(message, message_id, verifier, socket, recv_buf_size).await
        unimplemented!()
      }),
    )
    .into()
  }

  fn shutdown(&mut self) {
    self.is_shutdown = true
  }

  fn is_shutdown(&self) -> bool {
    self.is_shutdown
  }
}
impl<S: DnsUdpSocket + Send, MF> Stream for StateUdpStream<S, MF> {
  type Item = Result<(), ProtoError>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
    // self.inner.poll_next_unpin(cx)
    unimplemented!()
  }
}


pub struct StateUdpConnect<S: Send = UdpSocket> {
  inner: UdpClientConnect<S>,
  window: Duration,
}

impl<S: Send + Unpin> Future for StateUdpConnect<S> {
  type Output = Result<StateUdpStream<S>, ProtoError>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
    trace!("polling StateUdpConnect");
    self.inner.poll_unpin(cx).map_ok(|inner| {
      // StateUdpStream { inner, window: self.window }
      unimplemented!()
    })
  }
}
