// Typed HTTP + WebSocket client for the Local Rosaray Service
// (contracts/local-service-api.md, contracts/event-bus.md).
//
// Bootstrapping: this feature does not include a native launcher, so for
// now the frontend learns the service's ephemeral port + session token
// (printed to the service's stdout on boot) from the page URL:
//   http://localhost:5173/?rosarayPort=54321&rosaraySession=<token>
// `autoConnect()` reads these once at startup; `connect()` remains the
// programmatic entry point for any future launcher integration.

let baseUrl = null;
let sessionToken = null;
let currentPort = null;
let socket = null;
let reconnectAttempt = 0;
let reconnectTimer = null;
let eventListeners = new Set();
let stateListeners = new Set();
let currentState = 'connecting';

export function isConnected() {
  return baseUrl != null && sessionToken != null;
}

export function currentSessionState() {
  return currentState;
}

/** Records the service's ephemeral port + session token (FR-040/FR-042). */
export function connect(port, token) {
  baseUrl = `http://127.0.0.1:${port}`;
  sessionToken = token;
  currentPort = port;
  openEventSocket(port, token);
}

/**
 * Forces an immediate reconnect attempt, bypassing the exponential backoff
 * — used by the Explorer's manual "Retry" action once the automatic
 * reconnection procedure has given up (`unavailable`, FR-039).
 */
export function retryConnect() {
  if (currentPort == null || sessionToken == null) return false;
  reconnectAttempt = 0;
  openEventSocket(currentPort, sessionToken);
  return true;
}

/** Reads `?rosarayPort=&rosaraySession=` from the current page URL. */
export function autoConnect() {
  const params = new URLSearchParams(location.search);
  const port = params.get('rosarayPort');
  const token = params.get('rosaraySession');
  if (port && token) {
    connect(port, token);
    return true;
  }
  return false;
}

/** Typed Command/Query call against local-service-api.md. */
export async function request(method, path, body) {
  if (!isConnected()) throw new ServiceUnavailableError('not connected to the local service');
  let response;
  try {
    // FormData bodies (bundle import) must go out untouched so the browser
    // sets the multipart boundary itself.
    const isForm = typeof FormData !== 'undefined' && body instanceof FormData;
    response = await fetch(baseUrl + path, {
      method,
      headers: {
        'X-Rosaray-Session': sessionToken,
        ...(body !== undefined && !isForm ? { 'Content-Type': 'application/json' } : {}),
      },
      body: body === undefined ? undefined : isForm ? body : JSON.stringify(body),
    });
  } catch (err) {
    throw new ServiceUnavailableError(err.message);
  }

  const contentType = response.headers.get('content-type') || '';
  if (!response.ok) {
    let code = 'service_unavailable';
    let message = response.statusText;
    let details;
    if (contentType.includes('application/json')) {
      try {
        const parsed = await response.json();
        code = parsed?.error?.code || code;
        message = parsed?.error?.message || message;
        details = parsed?.error?.details;
      } catch {
        /* body wasn't the expected error shape; fall through with defaults */
      }
    }
    throw new ServiceError(code, message, response.status, details);
  }
  if (contentType.includes('application/json')) return response.json();
  return response.blob();
}

export class ServiceError extends Error {
  constructor(code, message, status, details) {
    super(message || code);
    this.code = code;
    this.status = status;
    // Feature 002: e.g. `{ findings: [...] }` for not_publishable/bundle_invalid,
    // or the on-disk revision for draft_conflict.
    this.details = details;
  }
}

export class ServiceUnavailableError extends ServiceError {
  constructor(message) {
    super('service_unavailable', message, 0);
  }
}

/** Fetches an artifact's binary content as an object URL for `<img src>`. */
export async function fetchArtifactObjectUrl(artifactId) {
  const blob = await request('GET', `/artifacts/${artifactId}/content`);
  return URL.createObjectURL(blob);
}

/**
 * Export Bundle (User Story 5). Builds a bundle for the given Dataset
 * Versions / Run Records under `credential`, then returns
 * `{ result, blob }` — the manifest response plus the encrypted bundle file.
 * The credential is sent once and never kept client-side.
 */
export async function exportBundle(datasetVersionIds, runRecordIds, credential) {
  const result = await request('POST', '/export-bundles', {
    dataset_version_ids: datasetVersionIds,
    run_record_ids: runRecordIds,
    credential,
  });
  const blob = await request('GET', `/export-bundles/${result.export_bundle_id}/content`);
  return { result, blob };
}

/** Imports a bundle file (a `File`/`Blob`) with its export credential. */
export async function importBundle(file, credential) {
  const form = new FormData();
  form.append('bundle', file);
  form.append('credential', credential);
  return request('POST', '/export-bundles/import', form);
}

// ---- Read-only Knowledge Base queries ----

const qs = (params) => {
  const q = new URLSearchParams();
  Object.entries(params || {}).forEach(([k, v]) => {
    if (v !== undefined && v !== null && v !== '') q.set(k, v);
  });
  const text = q.toString();
  return text ? `?${text}` : '';
};

export const kb = {
  listEntries: (filters) => request('GET', `/kb/entries${qs(filters)}`),
  getBundle: (kind, id, version) => request('GET', `/kb/${encodeURIComponent(kind)}/${encodeURIComponent(id)}/${encodeURIComponent(version)}`),
  getDraft: (kind, id) => request('GET', `/kb/${encodeURIComponent(kind)}/${encodeURIComponent(id)}/draft`),
};

/** Event names added by feature 002 (contracts/events.md). */
export const KB_EVENTS = Object.freeze([
  'kb_scan_progress',
  'kb_scan_complete',
  'kb_bundle_changed_externally',
  'extraction_progress',
  'extraction_complete',
  'extraction_cancelled',
  'preview_stale',
  'eligibility_changed',
]);

/**
 * Subscribes to one or more Event Bus event types; returns an unsubscribe
 * function. A client that misses an event recovers by re-issuing the matching
 * Query (events.md), so handlers should treat events as hints.
 */
export function onEventType(types, handler) {
  const wanted = new Set(Array.isArray(types) ? types : [types]);
  const listener = (event) => {
    if (wanted.has(event.type)) handler(event.payload, event);
  };
  eventListeners.add(listener);
  return () => eventListeners.delete(listener);
}

function wsUrl(port, token) {
  return `ws://127.0.0.1:${port}/events?session=${encodeURIComponent(token)}`;
}

/**
 * Subscribes to the Event Bus (event-bus.md). `onEvent(event)` fires per
 * message; `onStateChange(state)` fires on every `session_state_changed`
 * (including the reconnect procedure's own transitions). Returns an
 * unsubscribe function.
 */
export function subscribeEvents(onEvent, onStateChange) {
  if (onEvent) eventListeners.add(onEvent);
  if (onStateChange) stateListeners.add(onStateChange);
  return () => {
    eventListeners.delete(onEvent);
    stateListeners.delete(onStateChange);
  };
}

function setState(state) {
  currentState = state;
  stateListeners.forEach((fn) => fn(state));
}

function openEventSocket(port, token) {
  clearTimeout(reconnectTimer);
  setState('connecting');
  try {
    socket = new WebSocket(wsUrl(port, token));
  } catch {
    scheduleReconnect(port, token);
    return;
  }

  socket.onmessage = (msg) => {
    let event;
    try {
      event = JSON.parse(msg.data);
    } catch {
      return;
    }
    if (event.type === 'session_state_changed') {
      reconnectAttempt = 0;
      setState(event.payload.state);
    }
    eventListeners.forEach((fn) => fn(event));
  };

  // Reconnection procedure (event-bus.md): never keep showing prior data as
  // current on close/error — the caller must re-fetch on the next `ready`.
  socket.onclose = () => scheduleReconnect(port, token);
  socket.onerror = () => scheduleReconnect(port, token);
}

const MAX_RECONNECT_ATTEMPTS = 6;

function scheduleReconnect(port, token) {
  reconnectAttempt += 1;
  setState(reconnectAttempt > MAX_RECONNECT_ATTEMPTS ? 'unavailable' : 'connecting');
  if (reconnectAttempt > MAX_RECONNECT_ATTEMPTS) return;
  const delay = Math.min(1000 * 2 ** reconnectAttempt, 15000);
  reconnectTimer = setTimeout(() => openEventSocket(port, token), delay);
}
