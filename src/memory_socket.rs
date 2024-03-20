
use std::{collections::HashMap, sync::{atomic::Ordering, Arc}, time::Duration};

use hickory_server::resolver::Name;
use tokio::sync::Mutex;

use crate::{client::memory_rt::Packet, server::{DomainStats, Statistics}};

#[derive(Debug, Clone, Copy)]
pub enum PacketResult {
  Ok, Failed, Risk,
}

pub async fn memory_to_udp(
  sender: tokio::sync::mpsc::Sender<Packet>,
  mut receiver: tokio::sync::mpsc::Receiver<Packet>,
  stats: Arc<Statistics>,
  high_risk_domain: Arc<Mutex<HashMap<Name, DomainStats>>>,
) {
  // let mut udp = HashMap::new();
  let mut last_handles_report = tokio::time::Instant::now();
  let mut handles = Vec::new();
  while let Some(packet) = receiver.recv().await {
    let handle = tokio::spawn(udp_handle(packet, sender.clone(), stats.clone(), true));
    handles.push(handle);
    handles = handles.into_iter().filter(|i| !i.is_finished()).collect();
    if last_handles_report.elapsed() > Duration::from_secs(5) {
      let pending = stats.queries.load(Ordering::Relaxed).saturating_sub(stats.success.load(Ordering::Relaxed)).saturating_sub(stats.failed.load(Ordering::Relaxed));
      last_handles_report = tokio::time::Instant::now();
      if handles.len() > 5 {
        warn!(handles.len=handles.len(), pending, ?stats)
      } else {
        info!(handles.len=handles.len(), pending, ?stats)
      }
    } else {
      trace!(handles.len=handles.len())
    }
  }
}

async fn udp_handle(
  packet: Packet,
  sender: tokio::sync::mpsc::Sender<Packet>,
  stats: Arc<Statistics>,
  high_risk: bool,
) -> PacketResult {
  let packet = 'failed: loop {
    match packet {
      Packet::Udp { local_addr, remote_addr, buffer } => {
        stats.memory_recv.fetch_add(1, Ordering::Relaxed);
        let span = trace_span!("[memory]", port=local_addr.port()); let _span = span.enter();
        trace!(name: "sending", %local_addr, %remote_addr, len=buffer.len());
        let Ok(socket) = tokio::net::UdpSocket::bind(local_addr).await else {
          break 'failed Packet::Udp { local_addr, remote_addr, buffer: vec![] };
        };
        if socket.send_to(&buffer, remote_addr).await.is_err() {
          break 'failed Packet::Udp { local_addr, remote_addr, buffer: vec![] };
        }

        let decided = last_packet(Duration::from_secs(1), &socket, local_addr).await;
        if sender.send(Packet::Udp { local_addr, remote_addr, buffer: decided.unwrap_or_default() }).await.is_err() {
          stats.memory_sent.fetch_add(1, Ordering::Relaxed);
          return PacketResult::Ok;
        }
        stats.memory_sent.fetch_add(1, Ordering::Relaxed);
      },
    }

    return PacketResult::Ok;
  };
  stats.memory_failed.fetch_add(1, Ordering::Relaxed);
  sender.send(packet).await.ok();
  PacketResult::Failed
}

async fn last_packet(duration: Duration, socket: &tokio::net::UdpSocket, local_addr: std::net::SocketAddr) -> Option<Vec<u8>> {
  let mut decided = None;
  let mut out_buf = vec![0; 4096];
  let mut found = 0;
  let until = tokio::time::Instant::now().checked_add(duration).unwrap();
  loop {
    let sleep = tokio::time::sleep_until(until);
    tokio::select! {
      _ = sleep => {
        if found > 1 {
          info!(name: "finished", response_found=found);
        }
        trace!(name: "time up", response_found=found, overtime_ms=until.elapsed().as_millis());
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
    }
  }
  return decided;
}
