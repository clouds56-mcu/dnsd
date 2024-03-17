use hickory_server::{
  authority::Catalog,
  proto::rr::LowerName,
  resolver::{system_conf::parse_resolv_conf, Name},
  ServerFuture,
};

#[macro_use]
extern crate tracing;

pub mod client;
pub mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  dotenvy::dotenv().ok();
  tracing_subscriber::fmt::init();

  let conf = parse_resolv_conf("nameserver 1.1.1.1\n nameserver 8.8.8.8\n")?;
  info!("parsed config: {:?}", conf.0);
  info!("parsed opts: {:?}", conf.1);
  let authority = server::CheckedAuthority::new(conf);

  let listen_addr = std::env::var("DNSD_LISTEN").unwrap_or("0.0.0.0:5553".to_string());
  info!("DNSD_LISTEN={:?}", listen_addr);

  let listener = tokio::net::UdpSocket::bind(listen_addr).await?;
  let mut catalog = Catalog::new();
  catalog.upsert(LowerName::new(&Name::root()), authority.to_boxed());
  let mut server_future = ServerFuture::new(catalog);
  info!("listen on udp={}", listener.local_addr()?);
  server_future.register_socket(listener);
  server_future.block_until_done().await?;
  Ok(())
}
