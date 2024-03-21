use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

#[derive(Debug, Default)]
pub struct StatsPerDomain {
  pub queries: AtomicU64,
  pub success: AtomicU64,
  pub suspect: AtomicU64,
  pub failed: AtomicU64,
  pub empty: AtomicU64,
  pub refresh_needed: AtomicBool,
}

pub struct DomainStats(Mutex<HashMap<Name, StatsPerDomain>>);

impl DomainStats {
  pub fn new() -> Self {
    Self(Mutex::new(HashMap::new()))
  }

  pub fn try_get_count(&self) -> Option<usize> {
    let stats = self.0.try_lock().ok()?;
    Some(stats.len())
  }

  pub async fn check(&self, name: &Name) -> bool {
    let stats = self.0.lock().await;
    let entry = stats.get(name);
    if let Some(entry) = entry {
      if entry.refresh_needed.load(Ordering::SeqCst) {
        return true;
      }
      let suspect = entry.suspect.load(Ordering::Relaxed);
      let success = entry.success.load(Ordering::Relaxed);
      // at least 1/10 of query is suspect
      if suspect > success / 9 {
        return true;
      } else if suspect > 0 {
        debug!(action="check", %name, suspect, success);
      }
    }
    false
  }

  pub async fn refresh_needed(&self, name: &Name, reset: bool) -> bool {
    let stats = self.0.lock().await;
    let entry = stats.get(name);
    if let Some(entry) = entry {
      return entry.refresh_needed.fetch_and(!reset, Ordering::SeqCst);
    }
    false
  }

  pub async fn need_refresh(&self, name: &Name, value: bool) {
    let mut stats = self.0.lock().await;
    let entry = stats.entry(name.clone()).or_insert_with(StatsPerDomain::default);
    entry.refresh_needed.store(value, Ordering::SeqCst);
  }

  pub async fn add_query(&self, name: &Name, force: bool) -> u64 {
    let mut stats = self.0.lock().await;
    if !force && !stats.contains_key(name) { return 0 }
    let entry = stats.entry(name.clone()).or_insert_with(StatsPerDomain::default);
    entry.queries.fetch_add(1, Ordering::Relaxed)
  }

  pub async fn add_success(&self, name: &Name, force: bool) -> u64 {
    let mut stats = self.0.lock().await;
    if !force && !stats.contains_key(name) { return 0 }
    let entry = stats.entry(name.clone()).or_insert_with(StatsPerDomain::default);
    entry.success.fetch_add(1, Ordering::Relaxed) + 1
  }

  pub async fn add_suspect(&self, name: &Name) -> u64 {
    let mut stats = self.0.lock().await;
    let entry = stats.entry(name.clone()).or_insert_with(StatsPerDomain::default);
    entry.suspect.fetch_add(1, Ordering::Relaxed) + 1
  }

  pub async fn add_failed(&self, name: &Name) -> u64 {
    let mut stats = self.0.lock().await;
    let entry = stats.entry(name.clone()).or_insert_with(StatsPerDomain::default);
    entry.failed.fetch_add(1, Ordering::Relaxed) + 1
  }

  pub async fn add_empty(&self, name: &Name, force: bool) -> u64 {
    let mut stats = self.0.lock().await;
    if !force && !stats.contains_key(name) { return 0 }
    let entry = stats.entry(name.clone()).or_insert_with(StatsPerDomain::default);
    entry.empty.fetch_add(1, Ordering::Relaxed) + 1
  }
}

/// DNS Request Handler
pub struct CheckedAuthority {
  origin: LowerName,
  client: Arc<MemoryClient>,
  stats: Arc<Statistics>,
  // TODO: split to lockfree
  high_risk_domain: Arc<DomainStats>,
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
    let high_risk_domain = Arc::new(DomainStats::new());
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
    let force = self.high_risk_domain.refresh_needed(&name.clone().into(), true).await;
    let result = self.client.resolve(&name.to_string(), rtype, force).await?;
    if rtype == RecordType::A || rtype == RecordType::AAAA {
      if result.records().is_empty() {
        self.stats.empty.fetch_add(1, Ordering::Relaxed);
        self.high_risk_domain.add_empty(&name.clone().into(), false).await;
        info!(func="lookup", %name, ?rtype, records.len=?result.records().len(), elapsed=%now.elapsed().as_millis());
      } else {
        debug!(func="lookup", %name, ?rtype, records.len=?result.records().len(), record=%result.records()[0], elapsed=%now.elapsed().as_millis());
      }
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
    self.high_risk_domain.add_query(&request.query.name().clone().into(), false).await;
    let result = match self.lookup(request.query.name(), request.query.query_type(), lookup_options).await {
      Ok(result) => result,
      Err(e) => {
        self.stats.failed.fetch_add(1, Ordering::Relaxed);
        self.high_risk_domain.add_failed(&request.query.name().clone().into()).await;
        return Err(e)?
      }
    };
    self.stats.success.fetch_add(1, Ordering::Relaxed);
    self.high_risk_domain.add_success(&request.query.name().clone().into(), false).await;
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
