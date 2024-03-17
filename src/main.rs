use std::sync::Arc;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  dotenvy::dotenv().ok();
  tracing_subscriber::fmt::init();

  let (rt, sender, receiver) = client::MemoryRuntime::new();
  let client = client::Vanilla::from_conf(r#"
    nameserver 1.1.1.1
    nameserver 8.8.8.8
  "#, rt)?;
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
  Ok(())
}
