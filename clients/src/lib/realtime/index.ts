/** WebSocket / SSE client for presence, channel fanout, agent events */
export function connectRealtime(_token: string) {
  // TODO: wire to TT_REALTIME_BACKEND (Redis → Iggy)
  return { close: () => {} };
}
