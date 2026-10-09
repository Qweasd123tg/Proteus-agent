const request = (id, turn, start, input, cached, output, tools, origin = 'direct', model = 'gpt-5.6') => ({
  exchange_id: id, turn_id: turn, model: { provider: 'openai', model }, origin,
  started_at_ms: start, finished_at_ms: start + 4000 + output * 3, status: 'completed', finish_reason: 'stop',
  usage: { input_tokens: input, output_tokens: output, cached_input_tokens: cached, cache_creation_input_tokens: null, reasoning_output_tokens: Math.round(output / 3) },
  message_count: 2 + tools, tool_count: tools, reasoning_effort: 'medium', max_output_tokens: null,
});

export function createServices({ signal }) {
  const start = Date.now() - 18 * 60_000;
  const usage = { session_id: 'demo', revision: 7, latest_turn_id: 'turn-2', requests: [
    request('ex-1', 'turn-1', start, 18400, 0, 920, 3), request('ex-2', 'turn-1', start + 40_000, 24100, 17800, 1480, 2),
    request('ex-3', 'turn-1', start + 95_000, 31800, 23500, 2260, 0), request('ex-4', 'turn-2', start + 600_000, 38900, 31200, 1150, 4),
    request('ex-5', 'turn-2', start + 660_000, 52600, 37400, 640, 0, 'compactor', 'gpt-5.6-luna'), request('ex-6', 'turn-2', start + 700_000, 21300, 0, 1830, 1),
  ] };
  return { 'agent.usage.read': view => ({
    async read() { signal.throwIfAborted(); view.throwIfAborted(); return structuredClone(usage); },
  }) };
}
