use std::sync::Arc;

use hickory_server::{authority::{Authority, MessageResponseBuilder, ZoneType}, proto::op::{message, Header, ResponseCode}, server::{Request, RequestHandler, ResponseHandler, ResponseInfo}, store::in_memory::InMemoryAuthority};

use crate::client::Client;

/// DNS Request Handler
#[derive(Clone, Debug)]
pub struct Handler {
  client: Arc<Client>,
}

impl Handler {
  /// Create a new Server with the given client
  pub fn new(client: Arc<Client>) -> Self {
    Self { client }
  }
}

// fn get_domain_from_request(request: &Request) -> Option<String> {
//   match request {
//     Request::A { domain, .. } => Some(domain.clone()),
//     Request::AAAA { domain, .. } => Some(domain.clone()),
//     Request::PTR { domain, .. } => Some(domain.clone()),
//     Request::TXT { domain, .. } => Some(domain.clone()),
//     Request::MX { domain, .. } => Some(domain.clone()),
//     Request::NS { domain, .. } => Some(domain.clone()),
//     Request::SOA { domain, .. } => Some(domain.clone()),
//     Request::SRV { domain, .. } => Some(domain.clone()),
//     Request::ANY { domain, .. } => Some(domain.clone()),
//     _ => None,
//   }
// }

#[async_trait::async_trait]
impl RequestHandler for Handler {
  async fn handle_request<R: ResponseHandler>(
    &self,
    request: &Request,
    response: R,
  ) -> ResponseInfo {
    debug!("Handling request: {:?}", request);
    let id = request.header().id();
    let domain = request.query().name().to_string();
    info!("Resolving domain: [{id}] {}", domain);
    let lookup_resp = self.client.resolve(&domain, request.query().query_type()).await.unwrap();
    let builder = MessageResponseBuilder::from_message_request(&request);
    for record in lookup_resp.iter() {
      debug!("{}: {}", domain, record);
    }
    // response.send_response(message).await;
    let mut header = Header::new();
    header.set_response_code(ResponseCode::ServFail);
    ResponseInfo::from(header)
  }
}

#[cfg(test)]
mod tests {
  #[test]
  fn test_query() {
    let cmd = "dig";
    let args = &["@127.0.0.1", "-p5553", "www.example.com"];
    let output = std::process::Command::new(cmd).args(args).output().unwrap();
    let output = String::from_utf8_lossy(&output.stdout);
    println!("{}", output);
  }
}
