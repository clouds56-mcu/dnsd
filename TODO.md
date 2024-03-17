Roadmap
=======

1. multiple source/target
   * tcp/tls support
   * API
   * port forwarding
   * combine multiple result
   * ipv6 support
2. cache result
   * persist
   * if value changes, need new source is required
   * mark confidence, quick return on first show
3. communicate with other root
   * standard compilance
   * sync and domain registrar
   * other record_type
4. other protocol support
   * support for ddns (read and modify)
   * support for balance
   * used as service discover
   * lan and wireguard
   * mdns and non-ip devices
   * NAT pass through
   * GPS/satellite communication
5. blockchain support
   * discv4/discv5
   * ens integrate

Misc
=====
* UI support
   * show status and interest domain
   * config
* config file
   * interact with /etc/hosts
   * interact with /etc/resolve.conf
* system integrate
   * used on macOS without 53
   * used with systemd
   * used in (lightweight) container
