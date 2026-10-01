/**
 * Shiver companion plugin, client half. It runs in the page of everyone on the server, whether they
 * use Shiver or a browser.
 *
 * 1. A relay: Shiver's bridge posts named requests into the page and this forwards them to the
 *    plugin's server actions (same window, same origin only; unknown names are refused).
 * 2. Custom statuses, drawn in Sharkord's member list, profile card and user settings slots, and a
 *    button beside the settings gear to set your own.
 * 3. Usernames in their role colour, unless the server's admin turned that off.
 *
 * Plain JavaScript with no imports and no build step: Sharkord serves this file as it is.
 */

const REQUEST = 'shiver-bridge';
const RESPONSE = 'shiver-plugin';
const PLUGIN_ID = 'shiver';

const run = (store, name, payload) => store.actions.executePluginAction(PLUGIN_ID, name, payload);
const text = (value) => (typeof value === 'string' ? value : '');

/**
 * Sharkord imports this bundle by a url carrying the plugin's version, so an update made while a
 * page is open imports a second copy into it. The newest copy answers the relay and draws into the
 * page; older ones stand down.
 */
const COPY = {};

window.__SHIVER_PLUGIN_COPY__ = COPY;

const isCurrent = () => window.__SHIVER_PLUGIN_COPY__ === COPY;

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
    run(store, 'clearPushEndpoint', { endpoint: typeof payload?.endpoint === 'string' ? payload.endpoint : undefined })
};

const respond = (id, body) => window.postMessage({ source: RESPONSE, id, ...body }, window.location.origin);

window.addEventListener('message', async (event) => {
  if (event.source !== window || event.origin !== window.location.origin || !isCurrent()) return;

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

/**
 * Tells the bridge the plugin is here. Version 2: every write is a server action; adds setReadFloor.
 * Version 3: the plugin draws statuses and role colours itself, so the relay no longer sets or reads
 * a status.
 */
window.__SHIVER_PLUGIN__ = { version: 3 };

/* ── the page ── */

/* Sharkord's test ids, and class combinations where there is none. */
const SETTINGS_TRIGGER = '[data-testid="user-settings-trigger"]';
const MEMBER_ITEM = '[data-testid="member-item"]';
/** each message's wrapper; its parent's previous sibling is the author header */
const MESSAGE_WRAPPER = '[id^="message-"]';
/** the author's name in an inline reply's preview line */
const REPLY_AUTHOR = '[class~="max-w-40"][class~="truncate"][class~="font-medium"]';
/** `<span class="mention">@Name</span>` inside a message */
const MENTION_CHIP = 'span.mention';

/** The `<style>` element with `id`, created on first use. */
const ensureStyle = (id) => {
  const existing = document.getElementById(id);

  if (existing instanceof HTMLStyleElement) return existing;

  const style = document.createElement('style');

  style.id = id;
  document.head.append(style);

  return style;
};

const pageListeners = new Set();
/** elements added or re-texted since the last run; the body alone once there are too many to track */
let pageChanged = new Set();
let pageScheduled = false;
const MAX_TRACKED = 500;

const hasAncestorIn = (element, set) => {
  for (let up = element.parentElement; up; up = up.parentElement) if (set.has(up)) return true;

  return false;
};

const notePageChange = (node) => {
  const element = node instanceof Element ? node : node?.parentElement;

  if (!element || pageChanged.has(document.body)) return;

  if (pageChanged.size < MAX_TRACKED) pageChanged.add(element);
  else pageChanged = new Set([document.body]);
};

const runPageListeners = () => {
  pageScheduled = false;

  const changed = [...pageChanged].filter((element) => element.isConnected && !hasAncestorIn(element, pageChanged));

  pageChanged = new Set();

  for (const listener of pageListeners) {
    try {
      listener(changed);
    } catch (error) {
      console.error('[shiver] a page listener failed', error);
    }
  }
};

const pageObserver = new MutationObserver((records) => {
  if (!isCurrent()) return pageObserver.disconnect();

  for (const record of records) {
    if (record.type === 'characterData') notePageChange(record.target);
    else for (const node of record.addedNodes) notePageChange(node);
  }

  if (pageScheduled) return;

  pageScheduled = true;
  requestAnimationFrame(runPageListeners);
});

/**
 * Runs `listener` after the page changes, at most once a frame, with the outermost elements added or
 * re-texted since (`touched` finds what it cares about among them). Listeners must be idempotent.
 */
const onPageChange = (listener) => {
  if (!pageListeners.size) pageObserver.observe(document.body, { childList: true, subtree: true, characterData: true });

  pageListeners.add(listener);
};

/** Elements matching `selector` among, inside or enclosing (the nearest) the `changed` ones. */
const touched = (changed, selector) => {
  const found = new Set();

  for (const root of changed) {
    const around = root.closest(selector);

    if (around) found.add(around);

    for (const inner of root.querySelectorAll(selector)) found.add(inner);
  }

  return found;
};

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

/** Sets the user's own status (or clears it with ''), showing it here before the push arrives. */
const saveStatus = async (store, value) => {
  const status = text((await run(store, 'setStatus', { status: text(value) }))?.status);
  const ownUserId = store.getState().ownUserId;

  if (ownUserId !== undefined) applyStatus(ownUserId, status);

  return status;
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

/** Gives the popover status its own row above the buttons; degrades to inline without `:has()`. */
const ensurePopoverStatusStyle = () => {
  ensureStyle('shiver-popover-status-style').textContent = `
.${POPOVER_STATUS_CLASS} { flex-basis: 100%; min-width: 0; order: -1; text-align: left; }
div:has(> .${POPOVER_STATUS_CLASS}) { flex-wrap: wrap; max-width: 65%; }
`;
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

/** The status field in Sharkord's user settings; the button by the gear sets the same line. */
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
      setDraft(await saveStatus(store, draft));
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

/* ── the status button ── */

const STATUS_BUTTON_ID = 'shiver-status-button';
const STATUS_POPOVER_ID = 'shiver-status-popover';
const STATUS_BACKDROP_ID = 'shiver-status-backdrop';

/** lucide's `smile` */
const SMILE_ICON =
  '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" ' +
  'stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
  '<circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/>' +
  '<line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/></svg>';

const statusPopoverStyles = (touch) => `
#${STATUS_BACKDROP_ID} { position: fixed; inset: 0; z-index: 2147483646; background: rgb(0 0 0 / 45%); }
#${STATUS_POPOVER_ID} {
  position: fixed; z-index: 2147483647; box-sizing: border-box; display: flex; flex-direction: column;
  width: ${touch ? 'min(320px, calc(100vw - 32px))' : '280px'}; padding: ${touch ? 14 : 12}px;
  gap: ${touch ? 10 : 8}px; border-radius: ${touch ? 12 : 10}px;
  background: var(--popover, #1f1f1f); color: var(--popover-foreground, #fafafa);
  border: 1px solid var(--border, rgb(255 255 255 / 12%)); box-shadow: 0 12px 32px rgb(0 0 0 / 50%);
  font: 500 ${touch ? 14 : 13}px/1.4 system-ui, -apple-system, "Segoe UI", sans-serif;
}
#${STATUS_POPOVER_ID} label { color: var(--muted-foreground, #a1a1a1); font-size: 12px; }
#${STATUS_POPOVER_ID} input {
  width: 100%; box-sizing: border-box; padding: ${touch ? '10px' : '7px 9px'}; border-radius: ${touch ? 8 : 7}px;
  background: var(--input, rgb(255 255 255 / 6%)); color: inherit; font: inherit; outline: none;
  border: 1px solid var(--border, rgb(255 255 255 / 14%));
}
#${STATUS_POPOVER_ID} p { margin: 0; color: #f87171; font-size: 12px; }
#${STATUS_POPOVER_ID} div { display: flex; gap: ${touch ? 10 : 8}px; justify-content: flex-end; }
#${STATUS_POPOVER_ID} button {
  padding: ${touch ? '8px 16px' : '6px 12px'}; ${touch ? 'min-height: 40px;' : 'cursor: pointer;'}
  border-radius: ${touch ? 8 : 7}px; border: none; font: inherit;
  background: var(--primary, #e5e5e5); color: var(--primary-foreground, #171717);
}
#${STATUS_POPOVER_ID} button.ghost { background: transparent; color: var(--muted-foreground, #a1a1a1); }
`;

/** undoes what the open popover hooked into the page */
let releaseStatusPopover = null;

const closeStatusPopover = () => {
  releaseStatusPopover?.();
  releaseStatusPopover = null;
  document.getElementById(STATUS_POPOVER_ID)?.remove();
  document.getElementById(STATUS_BACKDROP_ID)?.remove();
};

/**
 * The "set your status" popover. On a touch screen it is centred on the visual viewport (above the
 * keyboard) over a backdrop; otherwise it opens above `anchor` and closes on a press outside it.
 */
const openStatusPopover = (store, anchor) => {
  if (document.getElementById(STATUS_POPOVER_ID)) return closeStatusPopover();

  const touch = window.matchMedia('(pointer: coarse)').matches;
  const host = document.createElement('div');
  const label = document.createElement('label');
  const input = document.createElement('input');
  const note = document.createElement('p');
  const row = document.createElement('div');
  const clear = document.createElement('button');
  const save = document.createElement('button');

  ensureStyle('shiver-status-style').textContent = statusPopoverStyles(touch);

  host.id = STATUS_POPOVER_ID;
  label.textContent = 'Your status — shown beside your name while you are online';
  input.maxLength = 100;
  input.placeholder = 'What are you up to?';
  input.enterKeyHint = 'done';
  note.hidden = true;
  clear.textContent = 'Clear';
  clear.className = 'ghost';
  save.textContent = 'Save';
  row.append(clear, save);
  host.append(label, input, note, row);

  if (touch) {
    const backdrop = document.createElement('div');
    const viewport = window.visualViewport;

    backdrop.id = STATUS_BACKDROP_ID;
    // on the backdrop, so the tap never reaches (and closes) the drawer underneath
    backdrop.addEventListener('pointerdown', (event) => {
      event.preventDefault();
      closeStatusPopover();
    });
    document.body.append(backdrop, host);

    // the keyboard shrinks the visual viewport after the popover opens, so this follows it
    const place = () => {
      const size = host.getBoundingClientRect();
      const width = viewport?.width ?? window.innerWidth;
      const height = viewport?.height ?? window.innerHeight;

      host.style.left = `${(viewport?.offsetLeft ?? 0) + Math.max(8, (width - size.width) / 2)}px`;
      host.style.top = `${(viewport?.offsetTop ?? 0) + Math.max(8, (height - size.height) / 2)}px`;
    };

    place();
    viewport?.addEventListener('resize', place);
    viewport?.addEventListener('scroll', place);
    releaseStatusPopover = () => {
      viewport?.removeEventListener('resize', place);
      viewport?.removeEventListener('scroll', place);
    };
  } else {
    document.body.append(host);

    const box = anchor.getBoundingClientRect();
    const size = host.getBoundingClientRect();

    host.style.left = `${Math.max(8, Math.min(box.left, window.innerWidth - size.width - 8))}px`;
    host.style.top = `${Math.max(8, box.top - size.height - 8)}px`;

    const outside = (event) => {
      if (!(event.target instanceof Node && host.contains(event.target))) closeStatusPopover();
    };

    // after this click has finished dispatching
    window.setTimeout(() => document.addEventListener('mousedown', outside, true), 0);
    releaseStatusPopover = () => document.removeEventListener('mousedown', outside, true);
  }

  input.focus();

  let typed = false;

  input.addEventListener('input', () => {
    typed = true;
  });

  // shown before the current status arrives; typing while it is in flight wins
  run(store, 'getOwnStatus').then(
    (current) => {
      if (!host.isConnected || typed) return;

      input.value = text(current?.status);
      input.select();
    },
    () => {}
  );

  const commit = async (value) => {
    save.disabled = clear.disabled = true;
    note.hidden = true;

    try {
      await saveStatus(store, value);
      closeStatusPopover();
    } catch (error) {
      note.textContent = error?.message || 'Could not save your status';
      note.hidden = false;
      save.disabled = clear.disabled = false;
    }
  };

  save.addEventListener('click', () => void commit(input.value));
  clear.addEventListener('click', () => void commit(''));
  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') void commit(input.value);
    if (event.key === 'Escape') closeStatusPopover();
  });
};

/** Puts the button beside Sharkord's settings gear, again whenever Sharkord redraws the panel. */
const addStatusButton = (store) => {
  if (document.getElementById(STATUS_BUTTON_ID)) return;

  const gear = document.querySelector(SETTINGS_TRIGGER);

  if (!gear) return;

  const button = document.createElement('button');

  button.id = STATUS_BUTTON_ID;
  button.type = 'button';
  button.className = gear.className;
  button.title = 'Set your status';
  button.setAttribute('aria-label', 'Set your status');
  button.innerHTML = SMILE_ICON;
  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    openStatusPopover(store, button);
  });

  gear.parentElement?.insertBefore(button, gear);
};

/* ── role colours ── */

/**
 * A user's colour: their first coloured non-default role in the server's list order (Sharkord has
 * no role hierarchy), else the default role's. `#ffffff` is Sharkord's "no colour".
 */
const roleColor = (user, roles) => {
  let fallback = null;

  for (const role of roles) {
    if (!user.roleIds?.includes(role.id)) continue;

    const color = text(role.color).trim();

    if (!color || ['#ffffff', '#fff'].includes(color.toLowerCase())) continue;
    if (!role.isDefault) return color;

    fallback ??= color;
  }

  return fallback;
};

/** Whether the server's admin lets names be coloured: off until the server says, so a server that turned it off never flashes. */
let colorsAllowed = false;
/** names and ids to colours, and the user's own name (a mention of them keeps Sharkord's colour) */
let colors = { byName: new Map(), byId: new Map(), ownName: '' };
let colorInputs = [];
let colorStamp = '';
/** member rows to re-render when the colours change */
const colorListeners = new Set();
/** whether any name has been coloured, so a server without role colours costs no walks of the page */
let painted = false;

/** Colours one name; only nodes this coloured are ever reset, and one already right is left alone. */
const paintNode = (node, color) => {
  if (!node || (node.dataset.shiverRole ?? '') === color) return;

  node.style.color = color;

  if (color) node.dataset.shiverRole = color;
  else delete node.dataset.shiverRole;
};

/**
 * Colours the names in `changed` and around it, or everywhere: message headers, reply previews and
 * mentions other than the user's own. They carry no user id, so they are matched by text. Member
 * rows are `MemberColor`'s, by id.
 */
const paintNames = (changed) => {
  if (!isCurrent() || (colors.byName.size === 0 && !painted)) return;

  painted = colors.byName.size > 0;

  const find = (selector) => (changed ? touched(changed, selector) : document.querySelectorAll(selector));
  const byText = (node, name = node?.textContent?.trim() ?? '') => paintNode(node, colors.byName.get(name) ?? '');

  for (const wrapper of find(MESSAGE_WRAPPER)) {
    byText(wrapper.parentElement?.previousElementSibling?.querySelector(':scope > span'));
  }

  for (const name of find(REPLY_AUTHOR)) byText(name);

  for (const chip of find(MENTION_CHIP)) {
    const name = chip.textContent?.trim().replace(/^@/, '') ?? '';

    if (name && name !== colors.ownName) byText(chip, name);
  }
};

/** Rebuilds the colours when the users, the roles or the admin's switch changed, then repaints. */
const readColors = (state) => {
  // the store hands back the same arrays until they change, and it changes with every message
  const inputs = [state.users, state.roles, state.ownUserId, colorsAllowed];

  if (inputs.every((input, index) => input === colorInputs[index])) return;

  colorInputs = inputs;

  const users = state.users ?? [];
  const byName = new Map();
  const byId = new Map();

  for (const user of colorsAllowed ? users : []) {
    const color = roleColor(user, state.roles ?? []);

    if (color) {
      byName.set(user.name, color);
      byId.set(user.id, color);
    }
  }

  const ownName = users.find((user) => user.id === state.ownUserId)?.name ?? '';
  const stamp = JSON.stringify([ownName, [...byName], [...byId]]);

  if (stamp === colorStamp) return;

  colorStamp = stamp;
  colors = { byName, byId, ownName };
  paintNames();

  for (const listener of colorListeners) listener();
};

/** Colours the member's name in their row. The slot renders inside the row, after the name. */
const MemberColor = ({ userId }) => {
  const React = window.__SHARKORD_REACT__;
  const anchor = React.useRef(null);
  const [, bump] = React.useState(0);

  React.useEffect(() => {
    const listener = () => bump((value) => value + 1);

    colorListeners.add(listener);

    return () => colorListeners.delete(listener);
  }, []);

  const color = colors.byId.get(userId) ?? '';

  React.useLayoutEffect(() => {
    const name = anchor.current?.closest(MEMBER_ITEM)?.querySelector(':scope > span');

    paintNode(name, color);

    return () => paintNode(name, '');
  }, [color]);

  return React.createElement('span', { ref: anchor, hidden: true });
};

/* ── starting ── */

/**
 * Draws into the page once the server has answered. The answer carries the admin's options, and
 * getting one at all means this user may use the plugin: Sharkord refuses its actions to anyone
 * without permission to use plugins, and draws none of its slots for them either.
 */
const start = async () => {
  const store = window.__SHARKORD_STORE__;

  if (!store) return;

  let options;

  try {
    options = await run(store, 'getOptions');
  } catch {
    return;
  }

  if (!isCurrent()) return;

  const allowColors = (allowed) => {
    colorsAllowed = allowed === true;
    readColors(store.getState());
  };

  allowColors(options?.roleColors);

  const stopWatching = store.subscribe(() => {
    if (isCurrent()) readColors(store.getState());
    else stopWatching();
  });

  store.actions.onPush?.(PLUGIN_ID, (data) => {
    if (data?.kind === 'options' && isCurrent()) allowColors(data.roleColors);
  });

  // an older copy's button would open that copy's popover
  document.getElementById(STATUS_BUTTON_ID)?.remove();
  addStatusButton(store);

  onPageChange((changed) => {
    addStatusButton(store);
    paintNames(changed);
  });
};

void start();

// Sharkord's slot ids, as strings so this bundle needs no imports or build step.
export const components = {
  member_list_item: [MemberStatus, MemberColor],
  user_popover: [PopoverStatus],
  user_settings: [StatusSetting]
};
