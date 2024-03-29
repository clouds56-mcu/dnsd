
use anyhow::Result;
use hickory_server::proto::rr::RecordType;
use hickory_server::resolver::config::*;
use hickory_server::resolver::error::ResolveError;
use hickory_server::resolver::lookup::Lookup;
use hickory_server::resolver::name_server::ConnectionProvider;
use hickory_server::resolver::system_conf::parse_resolv_conf;
use hickory_server::resolver::AsyncResolver;

pub struct Client<P: ConnectionProvider> {
  config: (ResolverConfig, ResolverOpts),
  resolver: AsyncResolver<P>,
}

impl<P: ConnectionProvider> std::fmt::Debug for Client<P> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Client")
      .field("resolver", &self.config)
      .finish()
  }
}

impl<P: ConnectionProvider> Client<P> {
  pub fn new(config: (ResolverConfig, ResolverOpts), provider: P) -> Self {
    let resolver = AsyncResolver::new(
      config.0.clone(),
      config.1.clone(),
      provider,
    );
    Self {
      config,
      resolver,
    }
  }
  pub fn from_conf(conf: &str, provider: P) -> Result<Self> {
    let config = parse_resolv_conf(conf.as_bytes())?;
    Ok(Self::new(config, provider))
  }

  pub async fn resolve(
    &self,
    domain: &str,
    record_type: RecordType,
    force: bool,
  ) -> Result<Lookup, ResolveError> {
    if force {
      error!(action="force lookup", domain, ?record_type);
      self.resolver.clear_cache();
    }
    let response = self.resolver.lookup(domain, record_type).await?;
    for i in response.record_iter() {
      trace!("resolve {}: {}", domain, i);
    }
    Ok(response)
  }
}

#[test]
fn test_resolve() {
  let resolver = hickory_server::resolver::Resolver::from_system_conf().unwrap();
  let response = resolver.lookup("www.example.com", RecordType::A).unwrap();

  for ip in response.iter() {
    println!("{}", ip);
  }
}
