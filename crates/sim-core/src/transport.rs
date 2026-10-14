//! Tick-bound reliable application transport over the scenario's directed links.
use super::*;

const FRAGMENT_BYTES: usize = 512;
const RETRY_TICKS: u64 = 5;
const MAX_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Queued,
    InTransit,
    Delivered,
    Acknowledged,
    Expired,
    Dropped,
    Unacknowledged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_id: Option<u64>,
    pub id: Uuid,
    pub state: DeliveryState,
    pub recipient: Uuid,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageOptions {
    pub hop_limit: u16,
    pub metadata: PacketMetadata,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Message {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<MessageOptions>,
    from: Uuid,
    to: Uuid,
    payload: Vec<u8>,
    expires: u64,
    next_attempt: u64,
    attempts: u8,
    received: BTreeSet<usize>,
    delivered: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Fragment {
    pub(super) message: Uuid,
    pub(super) index: usize,
    pub(super) route: Vec<Uuid>,
    pub(super) hop: usize,
    pub(super) ack: bool,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(super) struct Transport {
    pub(super) messages: BTreeMap<Uuid, Message>,
    pub(super) packets: BTreeMap<u64, Fragment>,
    pub(super) events: Vec<DeliveryEvent>,
    finished: BTreeSet<Uuid>,
}

impl Transport {
    pub(super) fn retention_statistics(&self) -> performance::RetentionStatistics {
        performance::RetentionStatistics {
            transport_messages: self.messages.len(),
            transport_fragments: self.packets.len(),
            transport_completed_ids: self.finished.len(),
            transport_events: self.events.len(),
            transport_payload_bytes: self
                .messages
                .values()
                .map(|m| m.payload.len())
                .sum::<usize>()
                + self.events.iter().map(|e| e.payload.len()).sum::<usize>(),
            ..Default::default()
        }
    }
}

impl Simulation {
    pub(super) fn is_fragment_event(&self, event: &NetworkEvent) -> bool {
        match event {
            NetworkEvent::TransmissionStarted { packet, .. }
            | NetworkEvent::PacketDelivered { packet, .. }
            | NetworkEvent::PacketDropped { packet, .. } => {
                self.transport.packets.contains_key(&packet.id().get())
            }
            NetworkEvent::DataReceived { .. } => false,
        }
    }

    pub fn has_message_route(&self, from: Uuid, to: Uuid) -> bool {
        self.message_route(from, to).is_some()
    }
    /// Acceptance means queued, never delivered. IDs are idempotency keys.
    pub fn send_message(
        &mut self,
        id: Uuid,
        from: Uuid,
        to: Uuid,
        payload: Vec<u8>,
        expires: u64,
    ) -> Result<(), String> {
        self.send_message_with_options(id, from, to, payload, expires, None)
    }

    /// Preserve scheduling metadata and the application route's forwarding budget.
    #[allow(clippy::too_many_arguments)]
    pub fn send_message_with_options(
        &mut self,
        id: Uuid,
        from: Uuid,
        to: Uuid,
        payload: Vec<u8>,
        expires: u64,
        options: Option<MessageOptions>,
    ) -> Result<(), String> {
        if self.transport.finished.contains(&id) || self.transport.messages.contains_key(&id) {
            self.audit_application(
                audit::Kind::DuplicateSuppressed,
                id.to_string(),
                from,
                payload.len(),
                None,
                None,
            );
            return Ok(());
        }
        if payload.len() > 1_048_576 {
            self.audit_application(
                audit::Kind::Rejected,
                id.to_string(),
                from,
                payload.len(),
                None,
                Some("message exceeds 1 MiB".into()),
            );
            return Err("message exceeds 1 MiB".into());
        }
        if expires <= self.tick() {
            self.audit_application(
                audit::Kind::Rejected,
                id.to_string(),
                from,
                payload.len(),
                None,
                Some("message already expired".into()),
            );
            return Err("message already expired".into());
        }
        if self.message_route(from, to).is_none() {
            self.audit_application(
                audit::Kind::Rejected,
                id.to_string(),
                from,
                payload.len(),
                None,
                Some("no configured communication route".into()),
            );
            return Err("no configured communication route".into());
        }
        self.audit_application(
            audit::Kind::Accepted,
            id.to_string(),
            from,
            payload.len(),
            None,
            None,
        );
        self.transport.messages.insert(
            id,
            Message {
                started_at_ns: None,
                options,
                from,
                to,
                payload,
                expires,
                next_attempt: self.tick(),
                attempts: 0,
                received: BTreeSet::new(),
                delivered: false,
            },
        );
        Ok(())
    }

    pub(super) fn has_pending_message_between(&self, from: Uuid, to: Uuid) -> bool {
        self.transport
            .messages
            .values()
            .any(|message| message.from == from && message.to == to)
    }

    pub fn drain_deliveries(&mut self) -> Vec<DeliveryEvent> {
        std::mem::take(&mut self.transport.events)
    }

    fn message_route(&self, from: Uuid, to: Uuid) -> Option<Vec<Uuid>> {
        let mut queue = VecDeque::from([vec![from]]);
        let mut visited = BTreeSet::from([from]);
        while let Some(path) = queue.pop_front() {
            let last = *path.last()?;
            if last == to {
                return Some(path);
            }
            let mut neighbors: Vec<_> = self
                .communications
                .outgoing_links
                .get(&last)
                .into_iter()
                .flatten()
                .map(|index| &self.communications.links[*index])
                .map(|link| link.to_entity_id)
                .collect();
            neighbors.sort();
            for next in neighbors {
                if visited.insert(next) {
                    let mut p = path.clone();
                    p.push(next);
                    if next == to {
                        return Some(p);
                    }
                    queue.push_back(p);
                }
            }
        }
        None
    }

    /// Link availability depends on geometry/interference at a virtual time, not queue occupancy.
    /// Snapshot it once for this tick's transport pass; never cache it across interference updates.
    fn usable_routes(&self) -> BTreeMap<(Uuid, Uuid), Vec<Uuid>> {
        let mut neighbors: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        let at = self.network_time();
        for link in &self.communications.links {
            if self
                .communications
                .simulator
                .transmission_metrics_at(link.channel_id.clone(), link.source_device_id.clone(), at)
                .is_ok_and(|metrics| metrics.available)
            {
                neighbors
                    .entry(link.from_entity_id)
                    .or_default()
                    .push(link.to_entity_id);
            }
        }
        for adjacent in neighbors.values_mut() {
            adjacent.sort();
            adjacent.dedup();
        }
        let mut routes = BTreeMap::new();
        for from in self.communications.entity_devices.keys().copied() {
            let mut queue = VecDeque::from([vec![from]]);
            let mut visited = BTreeSet::from([from]);
            while let Some(path) = queue.pop_front() {
                let last = *path.last().unwrap();
                for next in neighbors.get(&last).into_iter().flatten().copied() {
                    if visited.insert(next) {
                        let mut next_path = path.clone();
                        next_path.push(next);
                        queue.push_back(next_path);
                    }
                }
                routes.insert((from, last), path);
            }
        }
        routes
    }

    fn send_fragment(&mut self, fragment: Fragment) {
        let Some(message) = self.transport.messages.get(&fragment.message) else {
            return;
        };
        let bytes = if fragment.ack {
            vec![0; 32]
        } else {
            let start = fragment.index * FRAGMENT_BYTES;
            message.payload[start..(start + FRAGMENT_BYTES).min(message.payload.len())].to_vec()
        };
        let queued = if let Some(options) = message.options.clone() {
            if fragment.route.len().saturating_sub(1) > usize::from(options.hop_limit) {
                return;
            }
            self.queue_transmission_with_metadata(
                fragment.route[fragment.hop],
                fragment.route[fragment.hop + 1],
                bytes,
                options.hop_limit.saturating_sub(fragment.hop as u16),
                options.metadata,
            )
        } else {
            self.queue_transmission(
                fragment.route[fragment.hop],
                fragment.route[fragment.hop + 1],
                bytes,
            )
        };
        if let Ok(packet) = queued {
            let attempt = self.transport.messages[&fragment.message].attempts;
            self.annotate_audit_packet(
                packet.get(),
                audit::Context {
                    message_id: Some(fragment.message.to_string()),
                    fragment: Some(fragment.index),
                    attempt: Some(attempt),
                    hop: Some(fragment.hop),
                    acknowledgement: fragment.ack,
                    from: fragment.route.get(fragment.hop).copied(),
                    to: fragment.route.get(fragment.hop + 1).copied(),
                },
            );
            self.transport.packets.insert(packet.get(), fragment);
        } else {
            self.audit_application(
                audit::Kind::Dropped,
                fragment.message.to_string(),
                fragment.route[fragment.hop],
                0,
                None,
                Some("scheduling_failed".into()),
            );
        }
    }

    pub(super) fn advance_messages(&mut self) -> (f64, f64, f64) {
        let started = std::time::Instant::now();
        let mut retired = false;
        let routes = if self.transport.messages.is_empty() {
            BTreeMap::new()
        } else {
            self.usable_routes()
        };
        let tick = self.radio_tick();
        let ids: Vec<_> = self.transport.messages.keys().copied().collect();
        for id in ids {
            let m = &self.transport.messages[&id];
            if tick >= m.expires || (m.attempts >= MAX_ATTEMPTS && tick >= m.next_attempt) {
                let m = self.transport.messages.remove(&id).unwrap();
                self.audit_application(
                    if tick >= m.expires {
                        audit::Kind::Expired
                    } else {
                        audit::Kind::Unacknowledged
                    },
                    id.to_string(),
                    m.from,
                    0,
                    Some(m.attempts),
                    None,
                );
                self.transport.events.push(DeliveryEvent {
                    at_ns: None,
                    packet_id: None,
                    id,
                    state: if m.delivered {
                        DeliveryState::Unacknowledged
                    } else if tick >= m.expires {
                        DeliveryState::Expired
                    } else {
                        DeliveryState::Dropped
                    },
                    recipient: m.to,
                    payload: Vec::new(),
                });
                self.transport.finished.insert(id);
                retired = true;
                continue;
            }
            if tick < m.next_attempt {
                continue;
            }
            let from = m.from;
            let to = m.to;
            let delivered = m.delivered;
            let fragments = m.payload.len().div_ceil(FRAGMENT_BYTES).max(1);
            let m = self.transport.messages.get_mut(&id).unwrap();
            m.attempts += 1;
            m.next_attempt = tick + RETRY_TICKS;
            let attempt = m.attempts;
            let bytes = m.payload.len();
            self.audit_application(
                audit::Kind::Attempt,
                id.to_string(),
                if delivered { to } else { from },
                if delivered { 32 } else { bytes },
                Some(attempt),
                None,
            );
            if from == to {
                let m = self.transport.messages.remove(&id).unwrap();
                self.audit_application(
                    audit::Kind::Reassembled,
                    id.to_string(),
                    to,
                    m.payload.len(),
                    Some(m.attempts),
                    None,
                );
                self.audit_application(
                    audit::Kind::Acknowledged,
                    id.to_string(),
                    from,
                    0,
                    Some(m.attempts),
                    None,
                );
                self.transport.events.push(DeliveryEvent {
                    at_ns: None,
                    packet_id: None,
                    id,
                    state: DeliveryState::Delivered,
                    recipient: to,
                    payload: m.payload,
                });
                self.transport.events.push(DeliveryEvent {
                    at_ns: None,
                    packet_id: None,
                    id,
                    state: DeliveryState::Acknowledged,
                    recipient: from,
                    payload: Vec::new(),
                });
                self.transport.finished.insert(id);
            } else if delivered {
                if let Some(route) = routes.get(&(to, from)).cloned() {
                    self.send_fragment(Fragment {
                        message: id,
                        index: 0,
                        route,
                        hop: 0,
                        ack: true,
                    });
                }
            } else if let Some(route) = routes.get(&(from, to)).cloned() {
                for index in 0..fragments {
                    if self.transport.messages[&id].received.contains(&index) {
                        continue;
                    }
                    self.send_fragment(Fragment {
                        message: id,
                        index,
                        route: route.clone(),
                        hop: 0,
                        ack: false,
                    });
                }
            }
        }
        let sent = started.elapsed();
        let mut events = self
            .advance_network()
            .expect("validated network must advance");
        events.extend(std::mem::take(&mut self.pending_fragment_events));
        let advanced = started.elapsed();
        for event in events {
            let owned = self.is_fragment_event(&event);
            if !owned {
                self.pending_network_events.push(event);
                continue;
            }
            if let NetworkEvent::TransmissionStarted { at, packet, .. } = &event {
                if let Some(fragment) = self.transport.packets.get(&packet.id().get()) {
                    if let Some(message) = self.transport.messages.get_mut(&fragment.message) {
                        // Emit lifecycle timing only for opted-in metadata-aware C2
                        // traffic. Historical reliable messages keep their prior events.
                        if !fragment.ack
                            && message.options.is_some()
                            && message.started_at_ns.is_none()
                        {
                            message.started_at_ns = Some(at.as_nanos());
                            self.transport.events.push(DeliveryEvent {
                                at_ns: Some(at.as_nanos()),
                                packet_id: Some(packet.id().get()),
                                id: fragment.message,
                                state: DeliveryState::InTransit,
                                recipient: message.from,
                                payload: Vec::new(),
                            });
                        }
                    }
                }
                continue;
            }
            let (packet_id, delivered) = match event {
                NetworkEvent::PacketDelivered { packet, .. } => (packet.id().get(), true),
                NetworkEvent::PacketDropped { packet, .. } => (packet.id().get(), false),
                _ => continue,
            };
            let Some(mut f) = self.transport.packets.remove(&packet_id) else {
                continue;
            };
            if !delivered || !self.transport.messages.contains_key(&f.message) {
                continue;
            }
            if f.hop + 2 < f.route.len() {
                f.hop += 1;
                self.send_fragment(f);
                continue;
            }
            if f.ack {
                let m = self.transport.messages.remove(&f.message).unwrap();
                self.audit_application(
                    audit::Kind::Acknowledged,
                    f.message.to_string(),
                    m.from,
                    0,
                    Some(m.attempts),
                    None,
                );
                self.transport.events.push(DeliveryEvent {
                    at_ns: None,
                    packet_id: None,
                    id: f.message,
                    state: DeliveryState::Acknowledged,
                    recipient: m.from,
                    payload: Vec::new(),
                });
                self.transport.finished.insert(f.message);
                retired = true;
            } else {
                let (duplicate, terminal, attempt) = {
                    let m = self.transport.messages.get_mut(&f.message).unwrap();
                    (!m.received.insert(f.index), m.to, m.attempts)
                };
                if duplicate {
                    self.audit_application(
                        audit::Kind::DuplicateSuppressed,
                        f.message.to_string(),
                        terminal,
                        0,
                        Some(attempt),
                        None,
                    );
                }
                let m = self.transport.messages.get_mut(&f.message).unwrap();
                if !m.delivered
                    && m.received.len() == m.payload.len().div_ceil(FRAGMENT_BYTES).max(1)
                {
                    m.delivered = true;
                    self.transport.events.push(DeliveryEvent {
                        at_ns: None,
                        packet_id: None,
                        id: f.message,
                        state: DeliveryState::Delivered,
                        recipient: m.to,
                        payload: m.payload.clone(),
                    });
                    let (from, to, bytes, attempt) = (m.from, m.to, m.payload.len(), m.attempts);
                    self.audit_application(
                        audit::Kind::Reassembled,
                        f.message.to_string(),
                        to,
                        bytes,
                        Some(attempt),
                        None,
                    );
                    if let Some(route) = routes.get(&(to, from)).cloned() {
                        self.send_fragment(Fragment {
                            message: f.message,
                            index: 0,
                            route,
                            hop: 0,
                            ack: true,
                        });
                    }
                }
            }
        }
        // Retired fragments have no observable effects: both send and receipt paths check message
        // liveness. Remove them once per tick, not once for every completed message (quadratic).
        if retired {
            self.transport
                .packets
                .retain(|_, fragment| self.transport.messages.contains_key(&fragment.message));
        }
        let finished = started.elapsed();
        (
            sent.as_secs_f64() * 1000.0,
            (advanced - sent).as_secs_f64() * 1000.0,
            (finished - advanced).as_secs_f64() * 1000.0,
        )
    }
}

impl Simulation {
    pub fn cancel_application_message(&mut self, id: Uuid) {
        if self.transport.messages.remove(&id).is_some() {
            self.transport.finished.insert(id);
            self.transport
                .packets
                .retain(|_, fragment| fragment.message != id);
        }
    }
}
