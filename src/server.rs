use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use hickory_server::authority::{Authority, AuthorityObject, LookupError, LookupObject, LookupOptions, MessageRequest, UpdateResult, ZoneType};
use hickory_server::proto::rr::{LowerName, Record, RecordType};
use hickory_server::resolver::config::{ResolverConfig, ResolverOpts};
use hickory_server::resolver::{lookup::Lookup, Name};
use hickory_server::server::RequestInfo;
use tokio::sync::Mutex;

use crate::client::{self, MemoryClient, MemoryRuntime};
use crate::memory_socket::memory_to_udp;

#[derive(Debug, Default)]
pub struct Statistics {
  pub queries: AtomicU64,
  pub success: AtomicU64,
  pub failed: AtomicU64,
  pub empty: AtomicU64,
  pub memory_recv: AtomicU64,
  pub memory_failed: AtomicU64,
  pub memory_sent: AtomicU64,
}

pub struct DomainStats {
  pub queries: AtomicU64,
  pub success: AtomicU64,
  pub suspect: AtomicU64,
  pub failed: AtomicU64,
  pub empty: AtomicU64,
}

/// DNS Request Handler
pub struct CheckedAuthority {
  origin: LowerName,
  client: Arc<MemoryClient>,
  stats: Arc<Statistics>,
  // TODO: split to lockfree
  high_risk_domain: Arc<Mutex<HashMap<Name, DomainStats>>>,
  _handlers: Vec<tokio::task::JoinHandle<()>>,
}

impl CheckedAuthority {
  /// Create a new Server with the given client
  pub fn new(config: (ResolverConfig, ResolverOpts)) -> Self {
    let origin = LowerName::new(&Name::root());
    let (rt, sender, receiver) = MemoryRuntime::new();
    let client = client::Vanilla::new(config, rt);

    let stats = Arc::new(Statistics::default());
    let stats2 = stats.clone();
    let high_risk_domain = Arc::new(Mutex::new(HashMap::new()));
    let high_risk_domain2 = high_risk_domain.clone();

    // Start the UDP handler
    let handle_udp = tokio::spawn(async move { memory_to_udp(sender, receiver, stats2, high_risk_domain2).await });

    Self {
      client: Arc::new(client),
      origin,
      stats,
      high_risk_domain,
      _handlers: vec![handle_udp],
    }
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
    if result.records().is_empty() {
      self.stats.empty.fetch_add(1, Ordering::Relaxed);
      info!(func="lookup", %name, ?rtype, records.len=?result.records().len(), elapsed=%now.elapsed().as_millis());
    } else {
      debug!(func="lookup", %name, ?rtype, records.len=?result.records().len(), record=%result.records()[0], elapsed=%now.elapsed().as_millis());
    }
    Ok(LookupResult(result))
  }

  async fn search(
    &self,
    request: RequestInfo<'_>,
    lookup_options: LookupOptions,
  ) -> Result<Self::Lookup, LookupError> {
    // let _span = info_span!("search", name=%request.query.name(), rtype=?request.query.query_type()); let _span = _span.enter();
    self.stats.queries.fetch_add(1, Ordering::Relaxed);
    let result = self.lookup(request.query.name(), request.query.query_type(), lookup_options).await.map_err(|e| {
      self.stats.failed.fetch_add(1, Ordering::Relaxed); e
    })?;
    self.stats.success.fetch_add(1, Ordering::Relaxed);
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
