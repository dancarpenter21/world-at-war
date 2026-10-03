use super::*;
use sim_core::{NetworkEvent, NetworkTime, PacketMetadata};

pub(super) enum DeliveryAction {
    ExecuteIntent(AuthorizedIntent),
    ReceiveExecutionAck {
        intent_id: Uuid,
    },
    ReceiveImpactReport {
        recipient_unit_id: Uuid,
        report: sim_core::combat::ImpactReport,
    },
    ReceiveTrackReport {
        recipient_unit_id: Uuid,
        source_track: sim_core::Track,
    },
    ActivateRequest {
        request_id: Uuid,
        step: usize,
    },
    ExecuteRequest {
        request_id: Uuid,
        intent: AuthorizedIntent,
    },
}

fn tick_time_ns(tick: u64) -> u64 {
    tick.saturating_mul(sim_core::TICK_SECONDS)
        .saturating_mul(1_000_000_000)
}

fn flow_identity(role: Uuid, recipient: Uuid, profile: &str) -> u64 {
    role.as_bytes()
        .iter()
        .chain(recipient.as_bytes())
        .copied()
        .chain(profile.bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

pub(super) fn transmit_c2_message(
    game: &mut Game,
    origin_role_id: Uuid,
    recipient_entity_id: Uuid,
    profile_id: &str,
    rendered_text: String,
) -> Option<Uuid> {
    transmit_c2_message_with_fields(
        game,
        origin_role_id,
        recipient_entity_id,
        profile_id,
        rendered_text,
        BTreeMap::new(),
    )
}

pub(super) fn transmit_c2_message_with_fields(
    game: &mut Game,
    origin_role_id: Uuid,
    recipient_entity_id: Uuid,
    profile_id: &str,
    rendered_text: String,
    fields: BTreeMap<String, serde_json::Value>,
) -> Option<Uuid> {
    let origin_entity_id = game.roles.get(&origin_role_id)?.location_unit_id;
    transmit_c2_message_from_entity(
        game,
        (origin_role_id, origin_entity_id),
        recipient_entity_id,
        profile_id,
        rendered_text,
        fields,
    )
}

pub(super) fn transmit_c2_message_from_entity(
    game: &mut Game,
    (origin_role_id, origin_entity_id): (Uuid, Uuid),
    recipient_entity_id: Uuid,
    profile_id: &str,
    rendered_text: String,
    fields: BTreeMap<String, serde_json::Value>,
) -> Option<Uuid> {
    let profile = game.message_profiles.get(profile_id)?.clone();
    let tick = game.simulation.radio_tick();
    let message = C2Message {
        id: Uuid::new_v4(),
        profile_id: profile_id.into(),
        header: MessageHeader {
            origin_role_id,
            origin_entity_id,
            recipient_entity_id,
            classification: "simulation-controlled".into(),
            priority: profile.default_priority,
            created_tick: tick,
            expires_tick: tick.saturating_add(profile.expiry_ticks),
        },
        fields,
        rendered_text,
    };
    let encoded_bytes = message.encoded();
    let local = origin_entity_id == recipient_entity_id;
    let queued = if local {
        Ok(None)
    } else {
        game.simulation
            .queue_transmission_with_metadata(
                origin_entity_id,
                recipient_entity_id,
                encoded_bytes.clone(),
                profile.ttl_hops,
                PacketMetadata {
                    priority: profile.default_priority,
                    traffic_class: profile.default_priority / 64,
                    flow_id: flow_identity(origin_role_id, recipient_entity_id, profile_id),
                    expires_at: Some(NetworkTime::from_nanos(tick_time_ns(
                        message.header.expires_tick,
                    ))),
                },
            )
            .map(Some)
    };
    let (state, packet_id, drop_reason) = match queued {
        Ok(Some(id)) => (MessageState::Queued, Some(id.get()), None),
        Ok(None) => (MessageState::Delivered, None, None),
        Err(error) => (MessageState::Dropped, None, Some(error.to_string())),
    };
    let message_id = message.id;
    let record = NetworkMessageRecord {
        sequence: 0,
        message,
        encoded_bytes,
        state,
        packet_id,
        started_at_ns: None,
        terminal_at_ns: (state != MessageState::Queued).then_some(tick_time_ns(tick)),
        delivered_at_ns: local.then_some(tick_time_ns(tick)),
        drop_reason,
    };
    if !record_transition(game, record) {
        abort_pending(game, "network event store write failed");
        return None;
    }
    if state == MessageState::Dropped {
        return None;
    }
    if let Some(packet_id) = packet_id {
        game.packet_messages.insert(packet_id, message_id);
    }
    Some(message_id)
}

fn message_delivered(game: &Game, message_id: Uuid) -> bool {
    game.network_messages
        .iter()
        .any(|record| record.message.id == message_id && record.state == MessageState::Delivered)
}

pub(super) fn await_message_delivery(game: &mut Game, message_id: Uuid, action: DeliveryAction) {
    if message_delivered(game, message_id) {
        apply_delivery(game, message_id, action);
    } else {
        game.pending_deliveries.insert(message_id, action);
    }
}

pub(super) fn await_authority_delivery(
    game: &mut Game,
    request: &mut AuthorityRequest,
    message_id: Uuid,
    action: DeliveryAction,
) {
    request.status = AuthorityRequestStatus::InTransit { message_id };
    if message_delivered(game, message_id) {
        apply_to_request(game, request, action);
    } else {
        game.pending_deliveries.insert(message_id, action);
    }
}

fn apply_to_request(game: &mut Game, request: &mut AuthorityRequest, action: DeliveryAction) {
    match action {
        DeliveryAction::ActivateRequest { step, .. } if request.current_step == step => {
            if let Some(context) = &request.satellite_context {
                if context.authority_assignment.kind != SatelliteAuthorityKind::MilitaryRole {
                    request.status = AuthorityRequestStatus::PendingExternal {
                        authority_id: context.authority_assignment.authority_id.clone(),
                        resolves_at_tick: game
                            .simulation
                            .tick()
                            .saturating_add(EXTERNAL_OPERATOR_DELAY_TICKS),
                    };
                    return;
                }
            }
            if let Some(step) = request.policy.decision_steps.get(step) {
                request.status = status_for_decision_role(game, step, game.simulation.tick());
            } else {
                request.status = AuthorityRequestStatus::BlockedComms;
            }
        }
        DeliveryAction::ExecuteRequest { intent, .. } => {
            if !game.simulation.mission_complete() {
                game.simulation.queue_authorized_intent(intent);
            }
            request.status = AuthorityRequestStatus::Approved;
        }
        _ => {}
    }
}

fn apply_delivery(game: &mut Game, message_id: Uuid, action: DeliveryAction) {
    match action {
        DeliveryAction::ExecuteIntent(intent) if !game.simulation.mission_complete() => {
            game.simulation.queue_authorized_intent(intent)
        }
        DeliveryAction::ExecuteIntent(_) => {}
        DeliveryAction::ReceiveExecutionAck { intent_id } => {
            execution_acks::receive(game, message_id, intent_id);
        }
        DeliveryAction::ReceiveImpactReport {
            recipient_unit_id,
            report,
        } => {
            let received_tick = game
                .network_messages
                .iter()
                .find(|record| record.message.id == message_id)
                .and_then(|record| record.delivered_at_ns)
                .unwrap_or(0)
                .div_ceil(1_000_000_000);
            game.simulation
                .receive_impact_report(recipient_unit_id, report, received_tick);
        }
        DeliveryAction::ReceiveTrackReport {
            recipient_unit_id,
            source_track,
        } => {
            game.simulation
                .receive_track_report(recipient_unit_id, source_track);
        }
        action => {
            let request_id = match &action {
                DeliveryAction::ActivateRequest { request_id, .. }
                | DeliveryAction::ExecuteRequest { request_id, .. } => *request_id,
                DeliveryAction::ExecuteIntent(_)
                | DeliveryAction::ReceiveExecutionAck { .. }
                | DeliveryAction::ReceiveImpactReport { .. }
                | DeliveryAction::ReceiveTrackReport { .. } => {
                    unreachable!()
                }
            };
            let Some(mut request) = game.authority_requests.remove(&request_id) else {
                return;
            };
            if matches!(request.status, AuthorityRequestStatus::InTransit { message_id: awaiting } if awaiting == message_id)
            {
                apply_to_request(game, &mut request, action);
            }
            game.authority_requests.insert(request_id, request);
        }
    }
}

fn fail_delivery(game: &mut Game, message_id: Uuid, action: DeliveryAction) {
    let request_id = match action {
        DeliveryAction::ActivateRequest { request_id, .. }
        | DeliveryAction::ExecuteRequest { request_id, .. } => Some(request_id),
        DeliveryAction::ExecuteIntent(_)
        | DeliveryAction::ReceiveExecutionAck { .. }
        | DeliveryAction::ReceiveImpactReport { .. }
        | DeliveryAction::ReceiveTrackReport { .. } => None,
    };
    if let Some(request) = request_id.and_then(|id| game.authority_requests.get_mut(&id)) {
        if matches!(request.status, AuthorityRequestStatus::InTransit { message_id: awaiting } if awaiting == message_id)
        {
            request.status = AuthorityRequestStatus::BlockedComms;
        }
    }
    game.authority_events.push(AuthorityEvent {
        tick: game.simulation.tick(),
        kind: "message_delivery_failed".into(),
        detail: format!("message {message_id} did not reach its recipient"),
    });
}

pub(super) fn record_transition(game: &mut Game, mut record: NetworkMessageRecord) -> bool {
    game.network_event_sequence = game.network_event_sequence.saturating_add(1);
    game.network_projection_sequence = game.network_projection_sequence.saturating_add(1);
    record.sequence = game.network_event_sequence;
    let persisted = game.network_event_path.as_ref().is_none_or(|path| {
        serde_json::to_vec(&record).ok().is_some_and(|mut bytes| {
            bytes.push(b'\n');
            OpenOptions::new()
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(&bytes))
                .is_ok()
        })
    });
    if !persisted {
        record.state = MessageState::Dropped;
        record.delivered_at_ns = None;
        record.terminal_at_ns = Some(tick_time_ns(game.simulation.tick()));
        record.drop_reason = Some("network event store write failed".into());
        game.status = GameStatus::Paused;
        game.operational_error = Some("network event store write failed; game paused".into());
    }
    game.network_message_events.push(record.clone());
    if let Some(current) = game
        .network_messages
        .iter_mut()
        .find(|item| item.message.id == record.message.id)
    {
        *current = record;
    } else {
        game.network_messages.push(record);
    }
    persisted
}

fn abort_pending(game: &mut Game, reason: &str) {
    game.packet_messages.clear();
    for (message_id, action) in std::mem::take(&mut game.pending_deliveries) {
        fail_delivery(game, message_id, action);
    }
    let pending: Vec<_> = game
        .network_messages
        .iter()
        .filter(|record| matches!(record.state, MessageState::Queued | MessageState::InTransit))
        .cloned()
        .collect();
    for mut record in pending {
        game.network_event_sequence = game.network_event_sequence.saturating_add(1);
        game.network_projection_sequence = game.network_projection_sequence.saturating_add(1);
        record.sequence = game.network_event_sequence;
        record.state = MessageState::Dropped;
        record.terminal_at_ns = Some(tick_time_ns(game.simulation.tick()));
        record.drop_reason = Some(reason.into());
        game.network_message_events.push(record.clone());
        if let Some(current) = game
            .network_messages
            .iter_mut()
            .find(|item| item.message.id == record.message.id)
        {
            *current = record;
        }
    }
}

/// Close the finite post-mission window with persisted terminal packet states.
pub(super) fn finish_reporting(game: &mut Game) {
    let pending: Vec<_> = game
        .network_messages
        .iter()
        .filter(|record| matches!(record.state, MessageState::Queued | MessageState::InTransit))
        .cloned()
        .collect();
    for mut record in pending {
        let id = record.message.id;
        if let Some(packet) = record.packet_id {
            game.packet_messages.remove(&packet);
        }
        record.state = MessageState::Dropped;
        record.terminal_at_ns = Some(tick_time_ns(game.simulation.radio_tick()));
        record.drop_reason = Some("mission reporting window ended".into());
        if !record_transition(game, record) {
            abort_pending(game, "network event store write failed");
            return;
        }
        if let Some(action) = game.pending_deliveries.remove(&id) {
            fail_delivery(game, id, action);
        }
    }
}

pub(super) fn process_network_messages(game: &mut Game) {
    let events = match game.simulation.advance_network() {
        Ok(events) => events,
        Err(error) => {
            game.status = GameStatus::Paused;
            let reason = format!("network simulation failed: {error}");
            game.operational_error = Some(format!("{reason}; game paused"));
            abort_pending(game, &reason);
            return;
        }
    };
    for event in events {
        let (packet_id, at, state, reason) = match event {
            NetworkEvent::TransmissionStarted { at, packet, .. } => (
                packet.id().get(),
                at.as_nanos(),
                MessageState::InTransit,
                None,
            ),
            NetworkEvent::PacketDelivered { at, packet, .. } => (
                packet.id().get(),
                at.as_nanos(),
                MessageState::Delivered,
                None,
            ),
            NetworkEvent::PacketDropped {
                at, packet, reason, ..
            } => {
                let state = if matches!(reason, sim_core::CommunicationDropReason::Expired) {
                    MessageState::Expired
                } else {
                    MessageState::Dropped
                };
                (
                    packet.id().get(),
                    at.as_nanos(),
                    state,
                    Some(format!("{reason:?}")),
                )
            }
            NetworkEvent::DataReceived { .. } => continue,
        };
        let Some(message_id) = game.packet_messages.get(&packet_id).copied() else {
            continue;
        };
        let Some(mut record) = game
            .network_messages
            .iter()
            .find(|item| item.message.id == message_id)
            .cloned()
        else {
            continue;
        };
        if record.state == state {
            continue;
        }
        record.state = state;
        if state == MessageState::InTransit {
            record.started_at_ns = Some(at);
        }
        let terminal = matches!(
            state,
            MessageState::Delivered | MessageState::Dropped | MessageState::Expired
        );
        if terminal {
            record.terminal_at_ns = Some(at);
            record.drop_reason = reason;
            record.delivered_at_ns = (state == MessageState::Delivered).then_some(at);
            game.packet_messages.remove(&packet_id);
        }
        if !record_transition(game, record) {
            abort_pending(game, "network event store write failed");
            return;
        }
        if terminal {
            if let Some(action) = game.pending_deliveries.remove(&message_id) {
                if state == MessageState::Delivered {
                    apply_delivery(game, message_id, action);
                } else {
                    fail_delivery(game, message_id, action);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
