use std::{net::SocketAddr, str::FromStr, time::Duration};

use hickory_server::resolver::{config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts}, Name};

const DEFAULT_PORT: u16 = 53;

pub fn config_from_resolv(conf: resolv_conf::Config) -> anyhow::Result<(ResolverConfig, ResolverOpts)> {
  let domain = conf.get_system_domain().and_then(|d| Name::from_str(d.as_str()).ok());

  let name_servers = conf.nameservers.iter().map(|ip| {
    NameServerConfig {
      socket_addr: SocketAddr::new(ip.into(), DEFAULT_PORT),
      protocol: Protocol::Udp,
      tls_dns_name: None,
      trust_negative_responses: false,
      bind_addr: None,
    }
  }).collect::<Vec<_>>();

  let search = conf.get_last_search_or_domain()
    .filter(|d| d.as_str() != "--")
    .filter_map(|d| Name::from_str_relaxed(d.as_str()).ok())
    .collect::<Vec<_>>();

  let config = ResolverConfig::from_parts(domain, search, name_servers);

  let mut opts = ResolverOpts::default();
  opts.ndots = conf.ndots as usize;
  opts.timeout = Duration::from_secs(u64::from(conf.timeout));
  opts.attempts = conf.attempts as usize;

  Ok((config, opts))
}
