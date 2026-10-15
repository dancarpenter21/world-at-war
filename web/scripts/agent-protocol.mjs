// JSON schema shared by prompting and local validation. Model output contains proposals only.
const uuid = { type: 'string', pattern: '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$' };
const number = { type: 'number' };
const object = properties => ({ type: 'object', properties, required: Object.keys(properties), additionalProperties: false });
const variant = (name, properties) => object({ [name]: object(properties) });
const command = (action, properties = {}) => object({ action: { type: 'string', enum: [action] }, ...properties });
const interfaces = { type: 'string', enum: ['ground', 'boom', 'drogue', 'underway'] };
const stock = object({ fuel_kg: { ...number, minimum: 0 }, ammunition: { type: 'integer', minimum: 0 }, repair_points: { ...number, minimum: 0 } });
export const orderSchema = { anyOf: [
  variant('Move', { north_mps: number, east_mps: number }),
  variant('Engage', { track_id: uuid }),
  variant('SetRadarEmission', { emitting: { type: 'boolean' } }),
  variant('Logistics', { command: { anyOf: [command('return', { facility_id: uuid }), command('depart'), command('cancel'), command('support', { interface: interfaces }), command('refuel', { donor_id: uuid, interface: interfaces, amount_kg: { ...number, exclusiveMinimum: 0 } }), command('cargo', { source_id: uuid, destination_id: uuid, manifest: stock })] } }),
  variant('Iftu', { weapon_id: uuid, command: { anyOf: [command('update', { track_id: uuid }), command('retarget', { track_id: uuid }), command('assign_provider', { provider_id: uuid }), command('subscribe', { source_id: uuid, provider_id: uuid })] } }),
] };
export const proposalSchema = object({
  actions: { type: 'array', maxItems: 32, items: { anyOf: [
    object({ type: { type: 'string', enum: ['order'] }, unit_id: uuid, kind: orderSchema }),
    object({ type: { type: 'string', enum: ['select_goal'] }, goal_id: uuid, unit_ids: { type: 'array', maxItems: 128, items: uuid } }),
    object({ type: { type: 'string', enum: ['authority_decision'] }, request_id: uuid, approve: { type: 'boolean' } }),
  ] } },
  explanation: { type: 'string', maxLength: 2048 },
});

// Implement exactly the subset of JSON Schema used above, rejecting unknown fields.
export function matchesSchema(value, schema) {
  if (schema.anyOf) return schema.anyOf.some(s => matchesSchema(value, s));
  if (schema.enum && !schema.enum.includes(value)) return false;
  if (schema.type === 'object') return value !== null && typeof value === 'object' && !Array.isArray(value)
    && schema.required.every(k => Object.hasOwn(value, k))
    && Object.keys(value).every(k => Object.hasOwn(schema.properties, k))
    && Object.entries(schema.properties).every(([k, s]) => matchesSchema(value[k], s));
  if (schema.type === 'array') return Array.isArray(value) && value.length <= schema.maxItems && value.every(v => matchesSchema(v, schema.items));
  if (schema.type === 'string') return typeof value === 'string' && (schema.maxLength === undefined || value.length <= schema.maxLength) && (!schema.pattern || new RegExp(schema.pattern).test(value));
  if (schema.type === 'boolean') return typeof value === 'boolean';
  if (schema.type === 'number' || schema.type === 'integer') return typeof value === 'number' && Number.isFinite(value)
    && (schema.type !== 'integer' || Number.isInteger(value))
    && (schema.minimum === undefined || value >= schema.minimum)
    && (schema.exclusiveMinimum === undefined || value > schema.exclusiveMinimum);
  return false;
}
export function parseProposal(content) {
  if (typeof content !== 'string' || Buffer.byteLength(content) > 60_000) throw new Error('invalid_model_output');
  let result;
  try { result = JSON.parse(content); } catch { throw new Error('invalid_model_json'); }
  if (!matchesSchema(result, proposalSchema) || Buffer.byteLength(result.explanation) > 2048) throw new Error('invalid_model_schema');
  return result;
}
