//! Observational communications metadata. Never included in deterministic snapshots.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Accepted,
    DuplicateSuppressed,
    Attempt,
    Scheduled,
    TransmissionStarted,
    Received,
    Delivered,
    Dropped,
    Expired,
    Reassembled,
    Acknowledged,
    Unacknowledged,
    ReportingEnded,
    Rejected,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    pub message_id: Option<String>,
    pub fragment: Option<usize>,
    pub attempt: Option<u8>,
    pub hop: Option<usize>,
    pub acknowledgement: bool,
    pub from: Option<Uuid>,
    pub to: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub physical: Option<Physical>,
    pub kind: Kind,
    pub at_ns: u64,
    pub packet_id: Option<u64>,
    pub bytes: usize,
    pub context: Context,
    /// Only the terminal at which the observation occurred may inspect it.
    pub terminal: Option<Uuid>,
    pub reason: Option<String>,
}

/// Full physical routing metadata is private; role projections omit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Physical {
    pub source_device: String,
    pub destination_device: String,
    pub observer_device: String,
    pub channel: Option<String>,
    pub next_device: Option<String>,
    pub effective_bit_rate_bps: Option<u64>,
    pub receive_at_ns: Option<u64>,
    pub hops_remaining: u16,
}

#[derive(Default)]
pub(super) struct Audit {
    pub enabled: bool,
    pub events: Vec<Event>,
    pub packets: BTreeMap<u64, Context>,
}

impl Simulation {
    pub fn enable_audit(&mut self) {
        self.audit.enabled = true;
    }
    pub fn has_pending_audit(&self) -> bool {
        !self.audit.events.is_empty()
    }
    pub fn drain_audit(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.audit.events)
    }
    pub fn audit_correlations(&self) -> &BTreeMap<u64, Context> {
        &self.audit.packets
    }
    pub fn restore_audit_correlations(&mut self, packets: BTreeMap<u64, Context>) {
        self.audit.enabled = true;
        self.audit.events.clear();
        self.audit.packets = packets;
        // Existing snapshots already carry reliable-transport fragment associations.
        for (packet, f) in &self.transport.packets {
            self.audit
                .packets
                .entry(*packet)
                .or_insert_with(|| Context {
                    message_id: Some(f.message.to_string()),
                    fragment: Some(f.index),
                    hop: Some(f.hop),
                    acknowledgement: f.ack,
                    from: f.route.get(f.hop).copied(),
                    to: f.route.get(f.hop + 1).copied(),
                    ..Default::default()
                });
        }
        self.seed_iftu_audit();
    }
    pub fn active_audit_messages(&self) -> BTreeSet<String> {
        self.transport
            .messages
            .keys()
            .map(ToString::to_string)
            .chain(
                self.audit
                    .packets
                    .values()
                    .filter_map(|c| c.message_id.clone()),
            )
            .collect()
    }
    pub fn annotate_audit_packet(&mut self, packet: u64, context: Context) {
        if !self.audit.enabled {
            return;
        }
        self.audit.packets.insert(packet, context.clone());
        if let Some(event) = self
            .audit
            .events
            .iter_mut()
            .rev()
            .find(|e| e.packet_id == Some(packet) && e.kind == Kind::Scheduled)
        {
            event.context = context;
        }
    }
    pub(super) fn audit_application(
        &mut self,
        kind: Kind,
        message: String,
        terminal: Uuid,
        bytes: usize,
        attempt: Option<u8>,
        reason: Option<String>,
    ) {
        if !self.audit.enabled {
            return;
        }
        self.audit.events.push(Event {
            physical: None,
            kind,
            at_ns: self.network_time().as_nanos(),
            packet_id: None,
            bytes,
            context: Context {
                message_id: Some(message),
                attempt,
                ..Default::default()
            },
            terminal: Some(terminal),
            reason,
        });
    }
    pub(super) fn audit_scheduled(
        &mut self,
        packet: u64,
        from: Uuid,
        to: Uuid,
        bytes: usize,
        at_ns: u64,
    ) {
        if !self.audit.enabled {
            return;
        }
        let context = Context {
            from: Some(from),
            to: Some(to),
            ..Default::default()
        };
        self.audit.packets.insert(packet, context.clone());
        self.audit.events.push(Event {
            physical: None,
            kind: Kind::Scheduled,
            at_ns,
            packet_id: Some(packet),
            bytes,
            context,
            terminal: Some(from),
            reason: None,
        });
    }
    fn entity_for_audit_device(&self, device: &DeviceId) -> Option<Uuid> {
        self.communications
            .entity_devices
            .iter()
            .find_map(|(id, devices)| devices.contains(device).then_some(*id))
    }
    pub(super) fn audit_network_event(&mut self, event: &NetworkEvent) {
        if !self.audit.enabled {
            return;
        }
        let (kind, at, packet, device, reason, terminal_event) = match event {
            NetworkEvent::TransmissionStarted {
                at, packet, from, ..
            } => (Kind::TransmissionStarted, at, packet, from, None, false),
            NetworkEvent::DataReceived {
                at, packet, device, ..
            } => (Kind::Received, at, packet, device, None, false),
            NetworkEvent::PacketDelivered { at, packet, sink } => {
                (Kind::Delivered, at, packet, sink, None, true)
            }
            NetworkEvent::PacketDropped {
                at,
                packet,
                device,
                reason,
            } => (
                Kind::Dropped,
                at,
                packet,
                device,
                Some(drop_reason(reason).into()),
                true,
            ),
        };
        let context = self
            .audit
            .packets
            .get(&packet.id().get())
            .cloned()
            .unwrap_or_default();
        let terminal = self.entity_for_audit_device(device).or_else(|| {
            if device == packet.source() {
                context.from
            } else if device == packet.destination() {
                context.to
            } else {
                None
            }
        });
        let (channel, next_device, effective_bit_rate_bps, receive_at_ns) = match event {
            NetworkEvent::TransmissionStarted {
                channel,
                to,
                effective_bit_rate_bps,
                receive_at,
                ..
            } => (
                Some(channel.to_string()),
                Some(to.to_string()),
                Some(*effective_bit_rate_bps),
                Some(receive_at.as_nanos()),
            ),
            NetworkEvent::DataReceived {
                channel, device, ..
            } => (
                Some(channel.to_string()),
                Some(device.to_string()),
                None,
                None,
            ),
            _ => (None, None, None, None),
        };
        self.audit.events.push(Event {
            physical: Some(Physical {
                source_device: packet.source().to_string(),
                destination_device: packet.destination().to_string(),
                observer_device: device.to_string(),
                channel,
                next_device,
                effective_bit_rate_bps,
                receive_at_ns,
                hops_remaining: packet.hops_remaining(),
            }),
            kind,
            at_ns: at.as_nanos(),
            packet_id: Some(packet.id().get()),
            bytes: packet.payload().len(),
            context,
            terminal,
            reason,
        });
        if terminal_event {
            self.audit.packets.remove(&packet.id().get());
        }
    }
}

fn drop_reason(reason: &CommunicationDropReason) -> &'static str {
    use CommunicationDropReason::*;
    match reason {
        EndpointRetired => "endpoint_retired",
        ChannelSevered { .. } => "channel_severed",
        OutOfRange { .. } => "out_of_range",
        ReceiverInterference { .. } => "receiver_interference",
        QueueOverflow { .. } => "queue_overflow",
        MtuExceeded { .. } => "mtu_exceeded",
        ChannelLoss { .. } => "channel_loss",
        Expired => "expired",
        NoForwardingRule => "no_forwarding_rule",
        WrongDestination => "wrong_destination",
        UnsupportedDeviceRole => "unsupported_device_role",
        HopLimitExceeded => "hop_limit_exceeded",
    }
}
