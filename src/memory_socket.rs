
use std::{sync::{atomic::{AtomicUsize, Ordering}, Arc}, time::Duration};

use crate::{client::memory_rt::Packet, server::{DomainStats, Statistics}};

#[derive(Debug, Clone, Copy)]
pub enum PacketResult {
  Ok, Failed, Risk,
}

pub async fn memory_to_udp(
  sender: tokio::sync::mpsc::Sender<Packet>,
  mut receiver: tokio::sync::mpsc::Receiver<Packet>,
  stats: Arc<Statistics>,
  high_risk_domain: Arc<DomainStats>,
) {
  // let mut udp = HashMap::new();
  let mut last_handles_report = tokio::time::Instant::now();
  let mut handles = Vec::new();
  while let Some(packet) = receiver.recv().await {
    let handle = tokio::spawn(udp_handle(packet, sender.clone(), stats.clone(), high_risk_domain.clone()));
    let now = tokio::time::Instant::now();
    handles.push((handle, now.checked_add(Duration::from_secs(5)).unwrap()));
    handles = handles.into_iter().filter(|i| {
      let live = !i.0.is_finished() && i.1 > now;
      if !live { i.0.abort() }
      live
    }).collect();
    if last_handles_report.elapsed() > Duration::from_secs(5) {
      let pending = stats.queries.load(Ordering::Relaxed).saturating_sub(stats.success.load(Ordering::Relaxed)).saturating_sub(stats.failed.load(Ordering::Relaxed));
      last_handles_report = tokio::time::Instant::now();
      let high_risk_count = high_risk_domain.try_get_count();
      if handles.len() > 5 {
        info!(handles.len=handles.len(), pending, ?stats, ?high_risk_count)
      } else {
        debug!(handles.len=handles.len(), pending, ?stats, ?high_risk_count)
      }
    } else {
      trace!(handles.len=handles.len())
    }
  }
}

#[tracing::instrument(level = "info", skip_all, fields(%packet))]
async fn udp_handle(
  packet: Packet,
  sender: tokio::sync::mpsc::Sender<Packet>,
  stats: Arc<Statistics>,
  high_risk_domain: Arc<DomainStats>,
) -> PacketResult {
  let packet = 'failed: loop {
    match packet {
      Packet::Udp { local_addr, remote_addr, buffer, query } => {
        stats.memory_recv.fetch_add(1, Ordering::Relaxed);
        let query_name = query.as_ref().map(|i| &i.0);
        let span = trace_span!("[memory]", port=local_addr.port()); let _span = span.enter();
        trace!(name: "sending", %local_addr, %remote_addr, len=buffer.len());
        let Ok(socket) = tokio::net::UdpSocket::bind(local_addr).await else {
          break 'failed Packet::Udp { local_addr, remote_addr, buffer: vec![], query: None };
        };
        if socket.send_to(&buffer, remote_addr).await.is_err() {
          break 'failed Packet::Udp { local_addr, remote_addr, buffer: vec![], query: None };
        }

        let high_risk = if let Some(ref query) = query {
          high_risk_domain.check(&query.0).await
        } else {
          false
        };
        high_risk_domain.add_checking(query_name, false).await;
        // TODO: parse query from out_buf
        let decided = if high_risk {
          let (found, decided) = last_packet(Duration::from_secs(1), &socket, local_addr).await;
          if found > 1 {
            high_risk_domain.add_suspect(query_name).await;
          }
          decided
        } else {
          let mut out_buf = vec![0; 4096];
          let Ok((len, _)) = socket.recv_from(&mut out_buf).await else {
            break 'failed Packet::Udp { local_addr, remote_addr, buffer: vec![], query: None };
          };
          out_buf.truncate(len);
          Some(out_buf)
        };
        if sender.send(Packet::Udp { local_addr, remote_addr, buffer: decided.unwrap_or_default(), query: query.clone() }).await.is_err() {
          stats.memory_sent.fetch_add(1, Ordering::Relaxed);
          return PacketResult::Ok;
        }
        stats.memory_sent.fetch_add(1, Ordering::Relaxed);

        if !high_risk {
          // TODO: cache result
          let mut out_buf = vec![0; 4096];
          if socket.recv_from(&mut out_buf).await.is_ok() && query_name.is_some() {
            warn!(action="suspect after send", query=%query_name.unwrap());
            high_risk_domain.add_suspect(query_name).await;
            high_risk_domain.need_refresh(query_name.unwrap(), true).await;
          }
        }
      },
    }

    return PacketResult::Ok;
  };
  stats.memory_failed.fetch_add(1, Ordering::Relaxed);
  sender.send(packet).await.ok();
  PacketResult::Failed
}

async fn last_packet(duration: Duration, socket: &tokio::net::UdpSocket, local_addr: std::net::SocketAddr) -> (usize, Option<Vec<u8>>) {
  let mut decided = None;
  let mut out_buf = vec![0; 4096];
  let found = AtomicUsize::new(0);
  tokio::time::timeout(duration, async {
    while let Some((len, new_addr)) = socket.recv_from(&mut out_buf).await.ok() {
      trace!(name: "receiving", %new_addr, %local_addr, len);
      let out_buf = out_buf[..len].to_owned();
      decided = Some(out_buf);
      found.fetch_add(1, Ordering::SeqCst);
    }
  }).await.ok();
  let found = found.load(Ordering::SeqCst);
  if found > 1 {
    info!(action="last_packet", found);
  }
  return (found, decided);
}
