use std::{sync::Arc, time::Duration};

use client::memory_rt::Packet;
use hickory_server::{
  authority::{AuthorityObject, Catalog},
  proto::rr::LowerName,
  resolver::Name,
  ServerFuture,
};

#[macro_use]
extern crate tracing;

pub mod client;
pub mod server;

pub async fn memory_to_udp(sender: tokio::sync::mpsc::Sender<Packet>, mut receiver: tokio::sync::mpsc::Receiver<Packet>) {
  // let mut udp = HashMap::new();
  while let Some(packet) = receiver.recv().await {
    match packet {
      Packet::Udp { local_addr, remote_addr, buffer } => {
        debug!("[memory] sending {local_addr} {remote_addr} len={}", buffer.len());
        debug!("buffer: {:02x?}", buffer);
        let socket = tokio::net::UdpSocket::bind(local_addr).await.unwrap();
        socket.send_to(&buffer, remote_addr).await.unwrap();
        let mut out_buf = vec![0; 4096];
        let until = tokio::time::Instant::now().checked_add(Duration::from_secs(1)).unwrap();
        let mut decided = None;
        loop {
          let sleep = tokio::time::sleep_until(until);
          tokio::select! {
          _ = sleep => {
            warn!("[memory] finished");
            break;
          }
          _ = socket.readable() => {
            if let Ok((len, new_addr)) = socket.try_recv_from(&mut out_buf) {
              debug!("[memory] receiving {new_addr} {local_addr} len={}", len);
              let out_buf = out_buf[..len].to_owned();
              debug!("buffer: {:02x?}", out_buf);
              decided = Some(out_buf);
            }
          }
        } }
        if let Some(out_buf) = decided {
          sender.send(Packet::Udp { local_addr, remote_addr, buffer: out_buf }).await.unwrap();
        }
      },
    }
  }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  dotenvy::dotenv().ok();
  tracing_subscriber::fmt::init();

  let (rt, sender, receiver) = client::MemoryRuntime::new();
  let client = client::Vanilla::from_conf(r#"
    nameserver 8.8.8.8
  "#, rt)?;
  let handle = tokio::spawn(async move { memory_to_udp(sender, receiver).await });
  let authority = server::CheckedAuthority::new(Arc::new(client));
  let authority = Arc::new(authority);
  AuthorityObject::box_clone(&authority);

  let listener = tokio::net::UdpSocket::bind("0.0.0.0:5553").await?;
  let mut catalog = Catalog::new();
  catalog.upsert(LowerName::new(&Name::root()), Box::new(authority));
  let mut server_future = ServerFuture::new(catalog);
  info!("listen on udp={}", listener.local_addr()?);
  server_future.register_socket(listener);
  server_future.block_until_done().await?;
  drop(handle);
  Ok(())
}
