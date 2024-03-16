use std::sync::Arc;

use hickory_server::ServerFuture;

#[macro_use] extern crate tracing;

mod server;
mod client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  dotenvy::dotenv().ok();
  tracing_subscriber::fmt::init();

  let client = client::Client::from_conf(r#"
    nameserver 1.1.1.1
    nameserver 8.8.8.8
  "#)?;
  let handler = server::Handler::new(Arc::new(client));

  let listener = tokio::net::UdpSocket::bind("0.0.0.0:5553").await?;
  let mut server_future = ServerFuture::new(handler);
  info!("listen on udp={}", listener.local_addr()?);
  server_future.register_socket(listener);
  server_future.block_until_done().await?;
  Ok(())
}
