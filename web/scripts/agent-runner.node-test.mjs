import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration, infer, gameRequest, run } from './agent-runner.mjs';
import { parseProposal } from './agent-protocol.mjs';
const id = '00000000-0000-0000-0000-000000000001';
const env = { OPENROUTER_API_KEY: 'provider-secret', WAW_SERVER_URL: 'http://127.0.0.1:8000', WAW_AGENT_TOKEN: 'game-secret', WAW_GAME_ID: id };
const config = configuration(env);
const work = () => ({ protocol_version: 1, observation_id: id, assignment_generation: 2, deadline_unix_ms: Date.now() + 30000, projection: { own_units: [], tracks: [] } });
const answer = (content = '{"actions":[],"explanation":"Hold"}', extra = {}) => new Response(JSON.stringify({ model: 'test/model:free', choices: [{ finish_reason: 'stop', message: { content } }], ...extra }), { status: 200 });

test('configuration defaults to OpenRouter free and accepts an explicit free model/base URL', () => {
  assert.equal(config.baseUrl, 'https://openrouter.ai/api/v1'); assert.equal(config.model, 'openrouter/free');
  assert.equal(configuration({ ...env, OPENROUTER_MODEL: 'test/model:free', OPENROUTER_BASE_URL: 'http://localhost:9911/api/v1/' }).baseUrl, 'http://localhost:9911/api/v1');
  for (const model of ['openai/paid', 'openrouter/auto']) assert.throws(() => configuration({ ...env, OPENROUTER_MODEL: model }));
  for (const url of ['https://user:pass@example.com', 'http://remote.example/v1', 'https://example.com?key=x']) assert.throws(() => configuration({ ...env, OPENROUTER_BASE_URL: url }));
  assert.throws(() => configuration({ ...env, OPENROUTER_API_KEY: '' }));
});
test('provider request uses base URL, bearer key, schema and supplied observation only', async () => {
  let sent;
  const result = await infer(work(), config, { fetchImpl: async (url, init) => { sent = { url, ...init }; return answer(); } });
  assert.equal(sent.url, 'https://openrouter.ai/api/v1/chat/completions'); assert.equal(sent.headers.Authorization, 'Bearer provider-secret');
  const body = JSON.parse(sent.body); assert.equal(body.model, 'openrouter/free'); assert.equal(body.response_format.type, 'json_schema'); assert.equal(body.stream, false);
  assert.equal(JSON.parse(body.messages[1].content).assignment_generation, 2); assert.equal(result.model, 'test/model:free');
  assert.ok(!sent.body.includes('provider-secret')); assert.ok(!sent.body.includes('game-secret'));
});
test('unsupported structured output falls back to validated plain JSON once', async () => {
  const bodies = [];
  const result = await infer(work(), config, { fetchImpl: async (_, init) => { bodies.push(JSON.parse(init.body)); return bodies.length === 1 ? new Response(JSON.stringify({ error: { message: 'json_schema is not supported' } }), { status: 400 }) : answer(); } });
  assert.equal(bodies.length, 2); assert.ok(!bodies[1].response_format); assert.deepEqual(result.proposal.actions, []);
});
test('strict response format does not downgrade silently', async () => {
  await assert.rejects(infer(work(), { ...config, format: 'json_schema' }, { fetchImpl: async () => new Response('{}', { status: 400 }) }), /provider_unavailable/);
});
test('malformed, unknown, oversized and truncated model output cannot become decisions', async () => {
  for (const content of ['```json\n{}\n```', '{"actions":[{"type":"shell","command":"rm"}],"explanation":""}', '{"actions":[],"explanation":"","override":true}', 'x'.repeat(61000)]) {
    await assert.rejects(infer(work(), config, { fetchImpl: async () => answer(content) }), /invalid_model_output/);
  }
  await assert.rejects(infer(work(), config, { fetchImpl: async () => answer('{}', { choices: [{ finish_reason: 'length', message: { content: '{}' } }] }) }), /incomplete_model_output/);
});
test('local schema accepts existing movement, logistics, IFTU and authority actions', () => {
  const actions = [
    { type: 'order', unit_id: id, kind: { Move: { north_mps: 1, east_mps: 2 } } },
    { type: 'order', unit_id: id, kind: { Logistics: { command: { action: 'return', facility_id: id } } } },
    { type: 'order', unit_id: id, kind: { Iftu: { weapon_id: id, command: { action: 'update', track_id: id } } } },
    { type: 'select_goal', goal_id: id, unit_ids: [id] }, { type: 'authority_decision', request_id: id, approve: true },
  ];
  assert.deepEqual(parseProposal(JSON.stringify({ actions, explanation: '' })).actions, actions);
  assert.throws(() => parseProposal(JSON.stringify({ actions: [...actions, { type: 'order', unit_id: id, kind: { Move: { north_mps: 0, east_mps: 0, inject: true } } }], explanation: '' })));
});
test('rate limits respect Retry-After and never select a paid model', async () => {
  let calls = 0;
  await assert.rejects(infer(work(), config, { fetchImpl: async (_, init) => { calls++; assert.equal(JSON.parse(init.body).model, 'openrouter/free'); return new Response('{}', { status: 429, headers: { 'Retry-After': '60' } }); } }), error => error.message === 'provider_rate_limit' && error.retryAfterMs === 60000);
  assert.equal(calls, 1);
});
test('transient provider errors retry at most once within the deadline', async () => {
  let calls = 0; let waited = 0;
  await infer(work(), config, { wait: async ms => { waited += ms; }, fetchImpl: async () => ++calls === 1 ? new Response('{}', { status: 503 }) : answer() });
  assert.equal(calls, 2); assert.equal(waited, 1000);
});
test('authentication failures stop calls and quota exhaustion causes cooldown', async () => {
  for (const status of [401, 403]) await assert.rejects(infer(work(), config, { fetchImpl: async () => new Response('{"error":"provider-secret"}', { status }) }), e => e.fatal && e.message === 'provider_authentication');
  await assert.rejects(infer(work(), config, { fetchImpl: async () => new Response('{}', { status: 402 }) }), e => e.message === 'provider_quota' && e.retryAfterMs >= 300000);
});
test('expired work makes no inference request and timeout cancels a slow provider', async () => {
  await assert.rejects(infer({ ...work(), deadline_unix_ms: Date.now() - 1 }, config, { fetchImpl: async () => { assert.fail('must not call provider'); } }), /decision_expired/);
  await assert.rejects(infer(work(), { ...config, timeoutMs: 10 }, { fetchImpl: async (_, { signal }) => new Promise((_, reject) => { signal.addEventListener('abort', () => reject(new Error('provider-secret')), { once: true }); setTimeout(() => reject(new Error('test timeout')), 100); }) }), /inference_timeout/);
});
test('game calls use a separate credential and handle assignment revocation', async () => {
  await gameRequest(config, 'work', undefined, { fetchImpl: async (url, init) => { assert.equal(url, `http://127.0.0.1:8000/v1/games/${id}/agent/work`); assert.equal(init.headers.Authorization, 'Bearer game-secret'); assert.equal(init.redirect, 'error'); return new Response('{"work":null}'); } });
  await assert.rejects(gameRequest(config, 'work', undefined, { fetchImpl: async () => new Response('{}', { status: 401 }) }), e => e.fatal && e.message === 'game_authentication');
});
test('runner accepts a mock callback and retries the same decision after a lost submission response', async () => {
  const controller = new AbortController(); let inferred = 0; const submitted = []; const logs = [];
  await run(config, { signal: controller.signal, log: e => logs.push(e), decide: async () => { inferred++; return { proposal: { actions: [], explanation: 'Hold' }, model: 'mock/model:free' }; }, fetchImpl: async (url, init) => {
    if (url.endsWith('/work')) return new Response(JSON.stringify({ work: work() }));
    submitted.push(JSON.parse(init.body));
    if (submitted.length === 1) throw new Error('game-secret');
    controller.abort(); return new Response('{"results":[]}');
  } });
  assert.equal(inferred, 1); assert.equal(submitted.length, 2); assert.deepEqual(submitted[0], submitted[1]);
  assert.ok(!JSON.stringify(logs).includes('game-secret')); assert.ok(!JSON.stringify(logs).includes('provider-secret'));
});
