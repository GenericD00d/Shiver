/**
 * Shiver companion plugin, client half.
 *
 * 1. A relay: Shiver's bridge posts named requests into the page and this forwards them to the
 *    plugin's server actions (same window, same origin only; unknown names are refused).
 * 2. Custom statuses, drawn in Sharkord's member list, profile card and user settings slots.
 */

const REQUEST = 'shiver-bridge';
const RESPONSE = 'shiver-plugin';
const PLUGIN_ID = 'shiver';

const run = (store, name, payload) => store.actions.executePluginAction(PLUGIN_ID, name, payload);
const text = (value) => (typeof value === 'string' ? value : '');

/** Everything the relay will do. Each is a server action on the caller's own row. */
const ACTIONS = {
  getMutedChannels: (store) => run(store, 'getMutedChannels'),
  setMutedChannels: (store, payload) =>
    run(store, 'setMutedChannels', {
      mutedChannels: Array.isArray(payload?.mutedChannels) ? payload.mutedChannels : []
    }),
  setReadFloor: (store, payload) => run(store, 'setReadFloor', { floor: payload?.floor ?? null }),
  setPushEndpoint: (store, payload) => {
    const endpoint = text(payload?.endpoint);

    if (!/^https:\/\//i.test(endpoint)) throw new Error('A push endpoint must be https');

    return run(store, 'setPushEndpoint', { endpoint });
  },
  clearPushEndpoint: (store, payload) =>
    run(store, 'clearPushEndpoint', { endpoint: typeof payload?.endpoint === 'string' ? payload.endpoint : undefined }),
  getOwnStatus: async (store) => ({ status: text((await run(store, 'getOwnStatus'))?.status) }),
  setStatus: async (store, payload) => {
    const status = text((await run(store, 'setStatus', { status: text(payload?.status) }))?.status);
    const ownUserId = store.getState().ownUserId;

    // applied locally too: the server's push only reaches mounted components
    if (ownUserId !== undefined) applyStatus(ownUserId, status);

    return { status };
  }
};

const respond = (id, body) => window.postMessage({ source: RESPONSE, id, ...body }, window.location.origin);

window.addEventListener('message', async (event) => {
  if (event.source !== window || event.origin !== window.location.origin) return;

  const data = event.data;

  if (!data || data.source !== REQUEST || typeof data.id !== 'string') return;

  const action = Object.hasOwn(ACTIONS, data.action) ? ACTIONS[data.action] : null;
  const store = window.__SHARKORD_STORE__;

  if (!action) return respond(data.id, { ok: false, error: `Unknown action: ${data.action}` });
  if (!store) return respond(data.id, { ok: false, error: 'Sharkord store is not available' });

  try {
    respond(data.id, { ok: true, result: await action(store, data.payload) });
  } catch (error) {
    respond(data.id, { ok: false, error: error?.message ?? String(error) });
  }
});

/** Tells the bridge the plugin is here. Version 2: every write is a server action; adds setReadFloor. */
window.__SHIVER_PLUGIN__ = { version: 2 };

/* ── custom statuses ── */

/** Wraps and shrinks longer statuses, clamped to three lines. */
const statusStyle = (status) => ({
  fontSize: status.length <= 28 ? '12px' : status.length <= 56 ? '11px' : '10px',
  lineHeight: '1.3',
  whiteSpace: 'normal',
  overflowWrap: 'anywhere',
  display: '-webkit-box',
  WebkitBoxOrient: 'vertical',
  WebkitLineClamp: 3,
  overflow: 'hidden'
});

/** userId -> status for this page. */
const statuses = new Map();
/** userId -> Set of re-render callbacks, so a change wakes only the rows showing that user. */
const listeners = new Map();
/** Whether `statuses` reflects a completed load (reset when nothing is mounted to hear pushes). */
let loaded = false;
let loading = null;
/** Pushes heard while a load is in flight; newer than its answer. */
let heardDuringLoad = null;
/** Mounted components; only the first handles pushes, so each push is applied once. */
const pushOwners = new Set();

const announce = (userId) => {
  for (const listener of listeners.get(userId) ?? []) listener();
};

const announceAll = () => {
  for (const held of listeners.values()) for (const listener of held) listener();
};

const applyStatus = (userId, status) => {
  if (!Number.isInteger(userId)) return;

  if (status) statuses.set(userId, status);
  else statuses.delete(userId);

  heardDuringLoad?.set(userId, status);
  announce(userId);
};

/** Loads everyone's status once, replacing the map, then re-applies pushes heard meanwhile. */
const loadStatuses = (store) => {
  if (loaded) return Promise.resolve();
  if (loading) return loading;

  heardDuringLoad = new Map();
  loading = (async () => {
    try {
      const result = await run(store, 'getStatuses');
      const heard = heardDuringLoad;

      statuses.clear();

      for (const entry of result?.statuses ?? []) {
        if (Number.isInteger(entry?.userId) && text(entry.status)) statuses.set(entry.userId, entry.status);
      }

      for (const [userId, status] of heard) {
        if (status) statuses.set(userId, status);
        else statuses.delete(userId);
      }

      loaded = true;
      announceAll();
    } catch {
      // no statuses to show; the next mount asks again
    } finally {
      heardDuringLoad = null;
      loading = null;
    }
  })();

  return loading;
};

/** Subscribes a component to one user's status, loading the map if it may be stale. */
const useStatuses = (React, store, userId) => {
  const [, bump] = React.useState(0);
  const [token] = React.useState(() => ({}));

  React.useEffect(() => {
    // nothing was mounted, so pushes were missed: the map is stale
    if (listeners.size === 0) loaded = false;

    const held = listeners.get(userId) ?? new Set();
    const listener = () => bump((value) => value + 1);

    held.add(listener);
    listeners.set(userId, held);
    pushOwners.add(token);
    void loadStatuses(store);

    return () => {
      held.delete(listener);
      if (!held.size) listeners.delete(userId);
      pushOwners.delete(token);
    };
  }, [store, userId, token]);

  const onPush = React.useCallback(
    (data) => {
      if (data?.kind === 'status' && pushOwners.values().next().value === token) {
        applyStatus(data.userId, text(data.status));
      }
    },
    [token]
  );

  store.hooks.usePush(onPush);
};

/** Users by id, rebuilt only when Sharkord replaces the users array. */
let userIndex = { source: null, byId: new Map() };

const userById = (store, userId) => {
  const users = store.getState().users;

  if (!users) return undefined;

  if (userIndex.source !== users) userIndex = { source: users, byId: new Map(users.map((user) => [user.id, user])) };

  return userIndex.byId.get(userId);
};

const POPOVER_STATUS_CLASS = 'shiver-popover-status';
const POPOVER_STATUS_STYLE_ID = 'shiver-popover-status-style';

/** Gives the popover status its own row above the buttons; degrades to inline without `:has()`. */
const ensurePopoverStatusStyle = () => {
  if (document.getElementById(POPOVER_STATUS_STYLE_ID)) return;

  const style = document.createElement('style');

  style.id = POPOVER_STATUS_STYLE_ID;
  style.textContent = `
.${POPOVER_STATUS_CLASS} { flex-basis: 100%; min-width: 0; order: -1; text-align: left; }
div:has(> .${POPOVER_STATUS_CLASS}) { flex-wrap: wrap; max-width: 65%; }
`;
  document.head.append(style);
};

/** A status line for one user, shown only while they are online. */
const statusLine = (className, withStyle) =>
  function StatusLine({ userId }) {
    const React = window.__SHARKORD_REACT__;
    const store = window.__SHARKORD_STORE__;

    useStatuses(React, store, userId);
    React.useEffect(() => {
      if (withStyle) ensurePopoverStatusStyle();
    }, []);

    const status = statuses.get(userId);

    if (!status || userById(store, userId)?.status !== 'online') return null;

    return React.createElement('span', { className, style: statusStyle(status), title: status }, status);
  };

const MemberStatus = statusLine('ml-auto min-w-0 max-w-[45%] shrink text-muted-foreground', false);
const PopoverStatus = statusLine(`${POPOVER_STATUS_CLASS} text-muted-foreground`, true);

/** The status field in Sharkord's user settings: the route browser users have to set theirs. */
const StatusSetting = () => {
  const React = window.__SHARKORD_REACT__;
  const store = window.__SHARKORD_STORE__;
  const ownUserId = store.getState().ownUserId;

  useStatuses(React, store, ownUserId);

  const [draft, setDraft] = React.useState('');
  const [saving, setSaving] = React.useState(false);
  const [seeded, setSeeded] = React.useState(false);
  const [problem, setProblem] = React.useState('');

  // seeded once the load has finished (not on mount, when the map may still be empty)
  React.useEffect(() => {
    if (seeded || ownUserId === undefined || !loaded) return;

    setDraft(statuses.get(ownUserId) ?? '');
    setSeeded(true);
  });

  const save = async () => {
    setSaving(true);
    setProblem('');

    try {
      const { status } = await ACTIONS.setStatus(store, { status: draft });

      setDraft(status);
    } catch (error) {
      setProblem(error?.message ?? 'Could not save your status');
    } finally {
      setSaving(false);
    }
  };

  const h = React.createElement;

  return h(
    'div',
    { className: 'flex flex-col gap-2' },
    h('label', { className: 'text-sm font-medium', htmlFor: 'shiver-status' }, 'Status'),
    h(
      'p',
      { className: 'text-xs text-muted-foreground' },
      'A line shown beside your name while you are online. Everyone on this server can see it.'
    ),
    h(
      'div',
      { className: 'flex items-center gap-2' },
      h('input', {
        id: 'shiver-status',
        className: 'flex-1 rounded border border-input bg-transparent px-2 py-1.5 text-sm outline-none',
        value: draft,
        maxLength: 100,
        placeholder: 'What are you up to?',
        onChange: (event) => {
          setSeeded(true);
          setDraft(event.target.value);
        }
      }),
      h(
        'button',
        {
          type: 'button',
          className: 'rounded bg-primary px-3 py-1.5 text-sm text-primary-foreground',
          disabled: saving,
          onClick: () => void save()
        },
        saving ? 'Saving…' : 'Save'
      )
    ),
    problem ? h('p', { className: 'text-xs text-destructive', role: 'alert' }, problem) : null
  );
};

// Sharkord's slot ids, as strings so this bundle needs no imports or build step.
export const components = {
  member_list_item: [MemberStatus],
  user_popover: [PopoverStatus],
  user_settings: [StatusSetting]
};
