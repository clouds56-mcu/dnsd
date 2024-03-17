use std::{collections::HashMap, sync::Arc};

use client::memory_rt::Packet;
use hickory_server::{
  authority::{AuthorityObject, Catalog},
  proto::rr::LowerName,
  resolver::Name,
  ServerFuture,
};

#[macro_use]
extern crate tracing;

mod client;
mod server;

pub async fn memory_to_udp(sender: tokio::sync::mpsc::Sender<Packet>, mut receiver: tokio::sync::mpsc::Receiver<Packet>) {
  let mut udp = HashMap::new();
  while let Some(packet) = receiver.recv().await {
    match packet {
      Packet::Udp { local_addr, remote_addr, buffer } => {
        debug!("[memory] sending {local_addr} {remote_addr} len={}", buffer.len());
        debug!("buffer: {:02x?}", buffer);
        if !udp.contains_key(&local_addr) {
          udp.insert(local_addr, tokio::net::UdpSocket::bind(local_addr).await.unwrap());
        }
        let socket = udp.get(&local_addr).unwrap();
        socket.send_to(&buffer, remote_addr).await.unwrap();
        let mut out_buf = vec![0; 4096];
        // while let Ok((_, new_addr)) = socket.try_recv(&mut out_buf) {

        // }
        let (len, new_addr) = socket.recv_from(&mut out_buf).await.unwrap();
        out_buf.truncate(len);
        debug!("[memory] receiving {new_addr} {local_addr} len={}", len);
        debug!("buffer: {:02x?}", out_buf);
        sender.send(Packet::Udp { local_addr, remote_addr: new_addr, buffer: out_buf }).await.unwrap();
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
