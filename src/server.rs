use std::sync::Arc;
use std::time::Duration;

use hickory_server::authority::{Authority, AuthorityObject, LookupError, LookupObject, LookupOptions, MessageRequest, UpdateResult, ZoneType};
use hickory_server::proto::rr::{LowerName, Record, RecordType};
use hickory_server::resolver::config::{ResolverConfig, ResolverOpts};
use hickory_server::resolver::{lookup::Lookup, Name};
use hickory_server::server::RequestInfo;

use crate::client::memory_rt::Packet;
use crate::client::{self, MemoryClient, MemoryRuntime};

/// DNS Request Handler
pub struct CheckedAuthority {
  origin: LowerName,
  client: Arc<MemoryClient>,
  _handlers: Vec<tokio::task::JoinHandle<()>>,
}

impl CheckedAuthority {
  /// Create a new Server with the given client
  pub fn new(config: (ResolverConfig, ResolverOpts)) -> Self {
    let origin = LowerName::new(&Name::root());
    let (rt, sender, receiver) = MemoryRuntime::new();
    let client = client::Vanilla::new(config, rt);
    let handle = tokio::spawn(async move { memory_to_udp(sender, receiver).await });
    Self { client: Arc::new(client), origin, _handlers: vec![handle] }
  }

  pub fn to_boxed(self) -> Box<dyn AuthorityObject> {
    let authority = Arc::new(self);
    AuthorityObject::box_clone(&authority)
  }
}

#[async_trait::async_trait]
impl Authority for CheckedAuthority {
  type Lookup = LookupResult;
  fn zone_type(&self) -> ZoneType {
    ZoneType::Secondary
  }

  fn is_axfr_allowed(&self) -> bool {
    false
  }

  async fn update(&self, _update: &MessageRequest) -> UpdateResult<bool> {
    Ok(false)
  }

  fn origin(&self) -> &LowerName {
    &self.origin
  }

  async fn lookup(
    &self,
    name: &LowerName,
    rtype: RecordType,
    _lookup_options: LookupOptions,
  ) -> Result<Self::Lookup, LookupError> {
    let now = tokio::time::Instant::now();
    let result = self.client.resolve(&name.to_string(), rtype).await?;
    debug!(func="lookup", %name, ?rtype, records.len=?result.records().len(), elapsed=%now.elapsed().as_millis());
    Ok(LookupResult(result))
  }

  async fn search(
    &self,
    request: RequestInfo<'_>,
    lookup_options: LookupOptions,
  ) -> Result<Self::Lookup, LookupError> {
    let _span = info_span!("search", name=%request.query.name(), rtype=?request.query.query_type()); let _span = _span.enter();
    let result = self.lookup(request.query.name(), RecordType::A, lookup_options).await?;
    Ok(result)
  }

  async fn get_nsec_records(
    &self,
    name: &LowerName,
    lookup_options: LookupOptions,
  ) -> Result<Self::Lookup, LookupError> {
    info!(func="get_nsec_records", %name);
    let result = self.lookup(name, RecordType::NSEC, lookup_options).await?;
    Ok(result)
  }
}

pub struct LookupResult(Lookup);

impl LookupObject for LookupResult {
  fn is_empty(&self) -> bool {
    self.0.records().is_empty()
  }

  fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = &'a Record> + Send + 'a> {
    Box::new(self.0.record_iter())
  }

  fn take_additionals(&mut self) -> Option<Box<dyn LookupObject>> {
    None
  }
}


pub async fn memory_to_udp(sender: tokio::sync::mpsc::Sender<Packet>, mut receiver: tokio::sync::mpsc::Receiver<Packet>) {
  // let mut udp = HashMap::new();
  let mut handles = Vec::new();
  while let Some(packet) = receiver.recv().await {
    let handle = tokio::spawn(udp_handle(packet, sender.clone()));
    handles.push(handle);
    handles = handles.into_iter().filter(|i| !i.is_finished()).collect();
    if handles.len() > 5 {
      warn!(handles.len=handles.len())
    }
  }
}

async fn udp_handle(packet: Packet, sender: tokio::sync::mpsc::Sender<Packet>) {
  match packet {
    Packet::Udp { local_addr, remote_addr, buffer } => {
      let span = trace_span!("[memory]", port=local_addr.port()); let _span = span.enter();
      trace!(name: "sending", %local_addr, %remote_addr, len=buffer.len());
      let socket = tokio::net::UdpSocket::bind(local_addr).await.unwrap();
      socket.send_to(&buffer, remote_addr).await.unwrap();
      let mut out_buf = vec![0; 4096];
      let until = tokio::time::Instant::now().checked_add(Duration::from_secs(1)).unwrap();
      let mut decided = None;
      let mut found = 0;
      loop {
        let sleep = tokio::time::sleep_until(until);
        tokio::select! {
        _ = sleep => {
          info!(name: "finished", respone_found=found);
          break;
        }
        _ = socket.readable() => {
          if let Ok((len, new_addr)) = socket.try_recv_from(&mut out_buf) {
            trace!(name: "receiving", %new_addr, %local_addr, len);
            let out_buf = out_buf[..len].to_owned();
            decided = Some(out_buf);
            found += 1;
          }
        }
      } }
      if let Some(out_buf) = decided {
        sender.send(Packet::Udp { local_addr, remote_addr, buffer: out_buf }).await.unwrap();
      }
    },
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
