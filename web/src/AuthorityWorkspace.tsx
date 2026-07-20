import { useEffect, useMemo, useState } from "react";
import {
  addEdge, Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow,
  useEdgesState, useNodesState, type Connection, type Edge, type Node, type NodeProps
} from "@xyflow/react";
import ELK from "elkjs/lib/elk.bundled.js";
import "@xyflow/react/dist/style.css";

export type Side = "Blue" | "Red";
export type Unit = { id: string; name: string; domain: string };
export type Role = { id: string; name: string; side: Side; kind: string; location_unit_id: string; command_units: string[]; held: boolean; ai_controlled: boolean; lease_generation: number };
export type AuthorityRole = { id: string; name: string; side: Side; kind: string; location_unit_id: string; claimable: boolean; ai_controlled: boolean };
export type Relationship = { id: string; superior_role_id: string; subordinate_role_id?: string; subordinate_unit_id?: string; kind: string };
export type DecisionStep = { role_id: string; vacant_delay_ticks: number; approve_probability_bps: number };
export type Policy = { id: string; name: string; action: string; target_unit_ids: string[]; direct_role_ids: string[]; request_role_ids: string[]; decision_steps: DecisionStep[]; notify_role_ids: string[]; executable: boolean };
export type AuthorityDefinition = { version: number; roles: AuthorityRole[]; relationships: Relationship[]; policies: Policy[] };
export type AuthorityRequest = {
  id: string; action: string; target_unit_id: string; target: { kind: "unit"; unit_id: string } | { kind: "satellite"; norad_catalog_id: number }; requester_role_id: string; policy: Policy;
  policy_version: number; current_step: number; created_tick: number; summary: string;
  status: { state: string; role_id?: string; resolves_at_tick?: number }; decisions: { role_id: string; approved: boolean; automatic: boolean; tick: number }[];
};

const operationalKinds = new Set(["national_command", "cocom", "opcon", "tacon"]);
const relationshipColors: Record<string, string> = {
  national_command: "#e7c66c", cocom: "#e58b5f", opcon: "#57b6d6", tacon: "#5fc596",
  adcon: "#a98ad8", support: "#dba459", advisory: "#8d9aa2", transmit: "#d176a7"
};
const roleKinds = ["pilot", "national_command", "defense_secretary", "joint_staff", "combatant_commander", "joint_force_commander", "component_commander", "subordinate_commander", "tactical_commander"];

type AuthorityNodeData = { label: string; kind: string; side: Side; held: boolean; ai: boolean; unitCount: number };
function AuthorityNode({ data }: NodeProps<Node<AuthorityNodeData>>) {
  return <div className={`authority-node ${data.side.toLowerCase()}`}>
    <Handle type="target" position={Position.Top} />
    <small>{data.kind.replaceAll("_", " ")}</small><strong>{data.label}</strong>
    <span>{data.ai ? "AI" : data.held ? "PLAYER" : "VACANT"} · {data.unitCount} units</span>
    <Handle type="source" position={Position.Bottom} />
  </div>;
}
const nodeTypes = { authority: AuthorityNode };

async function layout(nodes: Node<AuthorityNodeData>[], edges: Edge[]): Promise<Node<AuthorityNodeData>[]> {
  const elk = new ELK();
  const graph = await elk.layout({ id: "root", layoutOptions: { "elk.algorithm": "layered", "elk.direction": "DOWN", "elk.spacing.nodeNode": "38", "elk.layered.spacing.nodeNodeBetweenLayers": "70" },
    children: nodes.map((node) => ({ id: node.id, width: 220, height: 84 })),
    edges: edges.map((edge) => ({ id: edge.id, sources: [edge.source], targets: [edge.target] })) });
  const positions = new Map(graph.children?.map((child) => [child.id, { x: child.x ?? 0, y: child.y ?? 0 }]));
  return nodes.map((node) => ({ ...node, position: positions.get(node.id) ?? node.position }));
}

export function AuthorityWorkspace({ definition, runtimeRoles, units, requests, currentRole, isHost, tick, onClose, onSave, onCreateRequest, onDecision }:
  { definition: AuthorityDefinition; runtimeRoles: Role[]; units: Unit[]; requests: AuthorityRequest[]; currentRole: Role | null; isHost: boolean; tick: number; onClose: () => void; onSave: (draft: AuthorityDefinition) => Promise<void>; onCreateRequest: (action: string, target: string, summary: string) => Promise<void>; onDecision: (requestId: string, decision: "approve" | "deny") => Promise<void> }) {
  const [draft, setDraft] = useState(() => structuredClone(definition));
  const [selectedRole, setSelectedRole] = useState(definition.roles[0]?.id ?? "");
  const [selectedPolicy, setSelectedPolicy] = useState(definition.policies[0]?.id ?? "");
  const [tab, setTab] = useState<"organization" | "policies" | "requests">("organization");
  const [saving, setSaving] = useState(false);
  const [summary, setSummary] = useState("Request space support for joint-force operations");
  const [requestPolicy, setRequestPolicy] = useState(definition.policies.find((policy) => policy.request_role_ids.includes(currentRole?.id ?? ""))?.id ?? "");
  const runtimeById = useMemo(() => new Map(runtimeRoles.map((role) => [role.id, role])), [runtimeRoles]);
  const roleNodes = useMemo<Node<AuthorityNodeData>[]>(() => draft.roles.map((role) => ({ id: role.id, type: "authority", position: { x: 0, y: 0 }, data: {
    label: role.name, kind: role.kind, side: role.side, held: runtimeById.get(role.id)?.held ?? false,
    ai: role.ai_controlled, unitCount: runtimeById.get(role.id)?.command_units.length ?? 0
  }})), [draft.roles, runtimeById]);
  const roleEdges = useMemo<Edge[]>(() => draft.relationships.filter((edge) => edge.subordinate_role_id).map((edge) => ({
    id: edge.id, source: edge.superior_role_id, target: edge.subordinate_role_id!, label: edge.kind.toUpperCase(),
    markerEnd: { type: MarkerType.ArrowClosed, color: relationshipColors[edge.kind] },
    style: { stroke: relationshipColors[edge.kind] ?? "#8899a2", strokeWidth: operationalKinds.has(edge.kind) ? 2 : 1.5, strokeDasharray: operationalKinds.has(edge.kind) ? undefined : "6 4" },
    labelStyle: { fill: "#9fb1ba", fontSize: 9 }
  })), [draft.relationships]);
  const [nodes, setNodes, onNodesChange] = useNodesState(roleNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(roleEdges);
  useEffect(() => { void layout(roleNodes, roleEdges).then(setNodes); setEdges(roleEdges); }, [roleNodes, roleEdges, setNodes, setEdges]);
  useEffect(() => setDraft(structuredClone(definition)), [definition]);

  const updateRole = (patch: Partial<AuthorityRole>) => setDraft((current) => ({ ...current, roles: current.roles.map((role) => role.id === selectedRole ? { ...role, ...patch } : role) }));
  const updatePolicy = (patch: Partial<Policy>) => setDraft((current) => ({ ...current, policies: current.policies.map((policy) => policy.id === selectedPolicy ? { ...policy, ...patch } : policy) }));
  const selectedRoleValue = draft.roles.find((role) => role.id === selectedRole);
  const selectedPolicyValue = draft.policies.find((policy) => policy.id === selectedPolicy);
  const eligiblePolicies = draft.policies.filter((policy) => currentRole && policy.request_role_ids.includes(currentRole.id));
  const selectedRequestPolicy = draft.policies.find((policy) => policy.id === requestPolicy);

  function connect(connection: Connection) {
    if (!isHost || !connection.source || !connection.target || connection.source === connection.target) return;
    const relationship: Relationship = { id: crypto.randomUUID(), superior_role_id: connection.source, subordinate_role_id: connection.target, kind: "opcon" };
    setDraft((current) => ({ ...current, relationships: [...current.relationships, relationship] }));
    setEdges((items) => addEdge({ ...connection, id: relationship.id }, items));
  }
  function addRole() {
    const id = crypto.randomUUID(); const location = units[0]?.id; if (!location) return;
    setDraft((current) => ({ ...current, roles: [...current.roles, { id, name: "New command role", side: "Blue", kind: "subordinate_commander", location_unit_id: location, claimable: true, ai_controlled: false }] }));
    setSelectedRole(id);
  }
  function deleteRole() {
    if (!selectedRole || runtimeById.get(selectedRole)?.held || runtimeById.get(selectedRole)?.ai_controlled) return;
    setDraft((current) => ({ ...current,
      roles: current.roles.filter((role) => role.id !== selectedRole),
      relationships: current.relationships.filter((edge) => edge.superior_role_id !== selectedRole && edge.subordinate_role_id !== selectedRole),
      policies: current.policies.map((policy) => ({ ...policy, direct_role_ids: policy.direct_role_ids.filter((id) => id !== selectedRole), request_role_ids: policy.request_role_ids.filter((id) => id !== selectedRole), notify_role_ids: policy.notify_role_ids.filter((id) => id !== selectedRole), decision_steps: policy.decision_steps.filter((step) => step.role_id !== selectedRole) }))
    }));
    setSelectedRole("");
  }
  function assignUnit(unitId: string, checked: boolean) {
    setDraft((current) => {
      const withoutOperationalAssignment = current.relationships.filter((edge) => !(edge.subordinate_unit_id === unitId && operationalKinds.has(edge.kind)));
      return { ...current, relationships: checked && selectedRole ? [...withoutOperationalAssignment, { id: crypto.randomUUID(), superior_role_id: selectedRole, subordinate_unit_id: unitId, kind: "tacon" }] : withoutOperationalAssignment };
    });
  }
  function addPolicy() {
    const id = crypto.randomUUID();
    setDraft((current) => ({ ...current, policies: [...current.policies, { id, name: "New authority policy", action: "move", target_unit_ids: [], direct_role_ids: [], request_role_ids: [], decision_steps: [], notify_role_ids: [], executable: true }] }));
    setSelectedPolicy(id); setTab("policies");
  }

  return <section className="authority-workspace">
    <header className="authority-header"><div><strong>AUTHORITY CONTROL</strong><span>Definition v{definition.version} · doctrine graph</span></div><button className="secondary" onClick={onClose}>Close</button></header>
    <nav className="authority-tabs">
      {(["organization", "policies", "requests"] as const).map((value) => <button className={tab === value ? "active" : ""} onClick={() => setTab(value)} key={value}>{value}</button>)}
      {isHost && <><button className="secondary" onClick={addRole}>Add role</button><button className="secondary" onClick={addPolicy}>Add policy</button><button className="command" disabled={saving} onClick={() => { setSaving(true); void onSave(draft).catch(() => undefined).finally(() => setSaving(false)); }}>{saving ? "Saving" : "Save live definition"}</button></>}
    </nav>
    {tab === "organization" && <div className="authority-layout"><div className="authority-canvas"><ReactFlow nodes={nodes} edges={edges} nodeTypes={nodeTypes} onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={connect} nodesConnectable={isHost} nodesDraggable={false} onNodeClick={(_, node) => setSelectedRole(node.id)} fitView><Background color="#314651" gap={20} /><MiniMap pannable zoomable /><Controls /></ReactFlow></div>
      <aside className="authority-inspector"><h2>Role inspector</h2>{selectedRoleValue ? <>
        <label>Name<input disabled={!isHost} value={selectedRoleValue.name} onChange={(event) => updateRole({ name: event.target.value })} /></label>
        <label>Role kind<select disabled={!isHost} value={selectedRoleValue.kind} onChange={(event) => updateRole({ kind: event.target.value })}>{roleKinds.map((kind) => <option key={kind}>{kind}</option>)}</select></label>
        <label>Command location<select disabled={!isHost} value={selectedRoleValue.location_unit_id} onChange={(event) => updateRole({ location_unit_id: event.target.value })}>{units.map((unit) => <option value={unit.id} key={unit.id}>{unit.name}</option>)}</select></label>
        <p className="muted">{runtimeById.get(selectedRoleValue.id)?.command_units.length ?? 0} units in computed operational scope.</p>
        <fieldset className="unit-assignments"><legend>Direct unit assignments</legend>{units.map((unit) => { const assigned = draft.relationships.some((edge) => edge.superior_role_id === selectedRole && edge.subordinate_unit_id === unit.id && operationalKinds.has(edge.kind)); return <label className="toggle" key={unit.id}><input type="checkbox" disabled={!isHost} checked={assigned} onChange={(event) => assignUnit(unit.id, event.target.checked)} />{unit.name}</label>; })}</fieldset>
        {isHost && <button className="danger" disabled={runtimeById.get(selectedRoleValue.id)?.held || runtimeById.get(selectedRoleValue.id)?.ai_controlled} onClick={deleteRole}>Delete unoccupied role</button>}
        <h2>Relationships</h2>{draft.relationships.filter((edge) => edge.superior_role_id === selectedRole || edge.subordinate_role_id === selectedRole).map((edge) => <div className="relationship-row" key={edge.id}><select disabled={!isHost} value={edge.kind} onChange={(event) => setDraft((current) => ({ ...current, relationships: current.relationships.map((item) => item.id === edge.id ? { ...item, kind: event.target.value } : item) }))}>{Object.keys(relationshipColors).map((kind) => <option key={kind}>{kind}</option>)}</select><button disabled={!isHost} onClick={() => setDraft((current) => ({ ...current, relationships: current.relationships.filter((item) => item.id !== edge.id) }))}>×</button></div>)}</> : <p className="muted">Select a role.</p>}</aside></div>}
    {tab === "policies" && <div className="policy-layout"><aside className="policy-list">{draft.policies.map((policy) => <button className={selectedPolicy === policy.id ? "selected" : ""} key={policy.id} onClick={() => setSelectedPolicy(policy.id)}><span>{policy.name}</span><small>{policy.action}</small></button>)}</aside><section className="policy-editor">{selectedPolicyValue && <>
      <label>Policy name<input disabled={!isHost} value={selectedPolicyValue.name} onChange={(event) => updatePolicy({ name: event.target.value })} /></label><div className="policy-grid"><label>Action key<input disabled={!isHost} value={selectedPolicyValue.action} onChange={(event) => updatePolicy({ action: event.target.value })} /></label><label className="toggle"><input disabled={!isHost} type="checkbox" checked={selectedPolicyValue.executable} onChange={(event) => updatePolicy({ executable: event.target.checked })} />Executable order</label></div>
      <label>Target units<select disabled={!isHost} multiple value={selectedPolicyValue.target_unit_ids} onChange={(event) => updatePolicy({ target_unit_ids: Array.from(event.target.selectedOptions, (option) => option.value) })}>{units.map((unit) => <option value={unit.id} key={unit.id}>{unit.name}</option>)}</select></label>
      <div className="policy-columns"><RoleChecks title="Direct authority" roles={draft.roles} selected={selectedPolicyValue.direct_role_ids} disabled={!isHost} onChange={(direct_role_ids) => updatePolicy({ direct_role_ids })} /><RoleChecks title="May request" roles={draft.roles} selected={selectedPolicyValue.request_role_ids} disabled={!isHost} onChange={(request_role_ids) => updatePolicy({ request_role_ids })} /></div>
      <h2>Ordered approval steps</h2>{selectedPolicyValue.decision_steps.map((step, index) => <div className="decision-step" key={`${step.role_id}-${index}`}><span>{index + 1}</span><select disabled={!isHost} value={step.role_id} onChange={(event) => updatePolicy({ decision_steps: selectedPolicyValue.decision_steps.map((item, i) => i === index ? { ...item, role_id: event.target.value } : item) })}>{draft.roles.map((role) => <option value={role.id} key={role.id}>{role.name}</option>)}</select><label>Vacant delay<input disabled={!isHost} type="number" min="0" value={step.vacant_delay_ticks} onChange={(event) => updatePolicy({ decision_steps: selectedPolicyValue.decision_steps.map((item, i) => i === index ? { ...item, vacant_delay_ticks: Number(event.target.value) } : item) })} /></label><label>Approve %<input disabled={!isHost} type="number" min="0" max="100" value={step.approve_probability_bps / 100} onChange={(event) => updatePolicy({ decision_steps: selectedPolicyValue.decision_steps.map((item, i) => i === index ? { ...item, approve_probability_bps: Number(event.target.value) * 100 } : item) })} /></label>{isHost && <button onClick={() => updatePolicy({ decision_steps: selectedPolicyValue.decision_steps.filter((_, i) => i !== index) })}>×</button>}</div>)}{isHost && <button className="secondary" onClick={() => { const roleId = draft.roles[0]?.id; if (roleId) updatePolicy({ decision_steps: [...selectedPolicyValue.decision_steps, { role_id: roleId, vacant_delay_ticks: 60, approve_probability_bps: 5000 }] }); }}>Add approval step</button>}</>}</section></div>}
    {tab === "requests" && <div className="requests-layout"><section><h2>New authority request</h2>{currentRole && eligiblePolicies.length ? <><label>Action policy<select value={requestPolicy} onChange={(event) => setRequestPolicy(event.target.value)}>{eligiblePolicies.map((policy) => <option value={policy.id} key={policy.id}>{policy.name}</option>)}</select></label><label>Target<select id="authority-target">{selectedRequestPolicy?.target_unit_ids.map((id) => <option value={id} key={id}>{units.find((unit) => unit.id === id)?.name ?? id}</option>)}</select></label><label>Request summary<textarea value={summary} maxLength={500} onChange={(event) => setSummary(event.target.value)} /></label><button className="command" onClick={() => { const target = (document.getElementById("authority-target") as HTMLSelectElement | null)?.value; if (selectedRequestPolicy && target) void onCreateRequest(selectedRequestPolicy.action, target, summary); }}>Send request</button></> : <p className="muted">This role has no requestable policies.</p>}</section><section><h2>Inbox and outbox</h2>{requests.length ? requests.map((request) => { const current = request.policy.decision_steps[request.current_step]; const actionable = currentRole?.id === current?.role_id && request.status.state === "pending_human"; return <article className="request-card" key={request.id}><div><strong>{request.policy.name}</strong><span className={`request-status ${request.status.state}`}>{request.status.state.replaceAll("_", " ")}</span></div><p>{request.summary || "Order approval request"}</p>{request.status.resolves_at_tick !== undefined && <small>Automatic decision in {Math.max(0, request.status.resolves_at_tick - tick)} ticks</small>}<small>{request.decisions.length} completed decisions · policy v{request.policy_version}</small>{actionable && <div className="request-actions"><button onClick={() => void onDecision(request.id, "deny")}>Deny</button><button className="command" onClick={() => void onDecision(request.id, "approve")}>Approve</button></div>}</article>; }) : <p className="muted">No authority traffic.</p>}</section></div>}
  </section>;
}

function RoleChecks({ title, roles, selected, disabled, onChange }: { title: string; roles: AuthorityRole[]; selected: string[]; disabled: boolean; onChange: (ids: string[]) => void }) {
  return <fieldset><legend>{title}</legend>{roles.map((role) => <label className="toggle" key={role.id}><input disabled={disabled} type="checkbox" checked={selected.includes(role.id)} onChange={(event) => onChange(event.target.checked ? [...selected, role.id] : selected.filter((id) => id !== role.id))} />{role.name}</label>)}</fieldset>;
}
