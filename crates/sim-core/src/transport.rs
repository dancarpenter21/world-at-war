//! Tick-bound reliable application transport over the scenario's directed links.
use super::*;

const FRAGMENT_BYTES: usize = 512;
const RETRY_TICKS: u64 = 5;
const MAX_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Queued,
    Delivered,
    Acknowledged,
    Expired,
    Dropped,
    Unacknowledged,
}

#[derive(Debug, Clone)]
pub struct DeliveryEvent {
    pub id: Uuid,
    pub state: DeliveryState,
    pub recipient: Uuid,
    pub payload: Vec<u8>,
}

struct Message {
    from: Uuid,
    to: Uuid,
    payload: Vec<u8>,
    expires: u64,
    next_attempt: u64,
    attempts: u8,
    received: BTreeSet<usize>,
    delivered: bool,
}

struct Fragment {
    message: Uuid,
    index: usize,
    route: Vec<Uuid>,
    hop: usize,
    ack: bool,
}

#[derive(Default)]
pub(super) struct Transport {
    messages: BTreeMap<Uuid, Message>,
    packets: BTreeMap<u64, Fragment>,
    pub(super) events: Vec<DeliveryEvent>,
    finished: BTreeSet<Uuid>,
}

impl Simulation {
    pub fn has_message_route(&self, from: Uuid, to: Uuid) -> bool {
        self.message_route(from, to, false).is_some()
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
        if self.transport.finished.contains(&id) || self.transport.messages.contains_key(&id) {
            return Ok(());
        }
        if payload.len() > 1_048_576 {
            return Err("message exceeds 1 MiB".into());
        }
        if expires <= self.tick() {
            return Err("message already expired".into());
        }
        if self.message_route(from, to, false).is_none() {
            return Err("no configured communication route".into());
        }
        self.transport.messages.insert(
            id,
            Message {
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

    pub fn drain_deliveries(&mut self) -> Vec<DeliveryEvent> {
        std::mem::take(&mut self.transport.events)
    }

    fn message_route(&self, from: Uuid, to: Uuid, usable_only: bool) -> Option<Vec<Uuid>> {
        let mut queue = VecDeque::from([vec![from]]);
        let mut visited = BTreeSet::from([from]);
        while let Some(path) = queue.pop_front() {
            let last = *path.last()?;
            if last == to {
                return Some(path);
            }
            let mut neighbors: Vec<_> = self
                .communications
                .links
                .iter()
                .filter(|link| link.from_entity_id == last)
                .filter(|link| {
                    !usable_only
                        || self
                            .communications
                            .simulator
                            .transmission_metrics_at(
                                link.channel_id.clone(),
                                link.source_device_id.clone(),
                                self.network_time(),
                            )
                            .is_ok_and(|metrics| metrics.available)
                })
                .map(|link| link.to_entity_id)
                .collect();
            neighbors.sort();
            for next in neighbors {
                if visited.insert(next) {
                    let mut p = path.clone();
                    p.push(next);
                    queue.push_back(p);
                }
            }
        }
        None
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
        if let Ok(packet) = self.queue_transmission(
            fragment.route[fragment.hop],
            fragment.route[fragment.hop + 1],
            bytes,
        ) {
            self.transport.packets.insert(packet.get(), fragment);
        }
    }

    pub(super) fn advance_messages(&mut self) {
        let tick = self.tick();
        let ids: Vec<_> = self.transport.messages.keys().copied().collect();
        for id in ids {
            let m = &self.transport.messages[&id];
            if tick >= m.expires || (m.attempts >= MAX_ATTEMPTS && tick >= m.next_attempt) {
                let m = self.transport.messages.remove(&id).unwrap();
                self.transport.events.push(DeliveryEvent {
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
                self.transport.packets.retain(|_, p| p.message != id);
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
            if from == to {
                let m = self.transport.messages.remove(&id).unwrap();
                self.transport.events.push(DeliveryEvent {
                    id,
                    state: DeliveryState::Delivered,
                    recipient: to,
                    payload: m.payload,
                });
                self.transport.events.push(DeliveryEvent {
                    id,
                    state: DeliveryState::Acknowledged,
                    recipient: from,
                    payload: Vec::new(),
                });
                self.transport.finished.insert(id);
            } else if delivered {
                if let Some(route) = self.message_route(to, from, true) {
                    self.send_fragment(Fragment {
                        message: id,
                        index: 0,
                        route,
                        hop: 0,
                        ack: true,
                    });
                }
            } else if let Some(route) = self.message_route(from, to, true) {
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
        let events = self
            .advance_network()
            .expect("validated network must advance");
        for event in events {
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
                self.transport.events.push(DeliveryEvent {
                    id: f.message,
                    state: DeliveryState::Acknowledged,
                    recipient: m.from,
                    payload: Vec::new(),
                });
                self.transport.finished.insert(f.message);
                self.transport.packets.retain(|_, p| p.message != f.message);
            } else {
                let m = self.transport.messages.get_mut(&f.message).unwrap();
                m.received.insert(f.index);
                if !m.delivered
                    && m.received.len() == m.payload.len().div_ceil(FRAGMENT_BYTES).max(1)
                {
                    m.delivered = true;
                    self.transport.events.push(DeliveryEvent {
                        id: f.message,
                        state: DeliveryState::Delivered,
                        recipient: m.to,
                        payload: m.payload.clone(),
                    });
                    let (from, to) = (m.from, m.to);
                    if let Some(route) = self.message_route(to, from, true) {
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
    }
}
