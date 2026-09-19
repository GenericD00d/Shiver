/**
 * Shiver companion plugin, client half.
 *
 * This exists only to be a relay. Shiver's bridge cannot reach a plugin's storage itself: the calls
 * live on Sharkord's plugin store, and only code served out of `/plugin-bundle/shiver/` is imported
 * as this plugin. That is this file.
 *
 * It answers messages posted inside the same page and nothing else, so it adds no surface a server
 * or another origin can reach. The two names below are the whole vocabulary — the relay cannot be
 * talked into reading or writing anything else, which is why it takes named operations rather than
 * passing storage calls straight through.
 */

const REQUEST = 'shiver-bridge';
const RESPONSE = 'shiver-plugin';

/**
 * This plugin's id, which the host wants named on every call.
 *
 * Sharkord used to read it off the call stack — the frame had to come from `/plugin-bundle/shiver/`
 * — and since 0.0.25 it is passed outright.
 */
const PLUGIN_ID = 'shiver';

/** Guards against a page sending an unbounded list into the user's row. */
const MAX_MUTED_CHANNELS = 500;

/** A value read as a mute list, keeping only what a channel id can be. */
const mutedFrom = (value) =>
  Array.isArray(value)
    ? [...new Set(value.filter((id) => Number.isInteger(id) && id > 0))].slice(0, MAX_MUTED_CHANNELS)
    : [];

/**
 * What the relay will do, and all it will do.
 *
 * Both sit on the host's per-user storage, which since 0.0.25 is a row per plugin per user: size
 * capped by the server, deleted when the user is deleted and when the plugin is removed. Shiver used
 * to keep a file and two server actions for this; the host does it better, and the row is the
 * user's own, so nothing here can reach anybody else's settings.
 */
const ACTIONS = {
  getMutedChannels: async (store) => {
    const stored = await store.actions.getUserData(PLUGIN_ID);

    return { mutedChannels: mutedFrom(stored?.mutedChannels) };
  },
  setMutedChannels: async (store, payload) => {
    const stored = await store.actions.getUserData(PLUGIN_ID);
    const mutedChannels = mutedFrom(payload?.mutedChannels);

    // The whole row is replaced by a write, so it is read first and spread: a Shiver that only knows
    // about muted channels must not wipe a setting a later one stored beside them.
    await store.actions.setUserData(PLUGIN_ID, { ...stored, mutedChannels });

    return { mutedChannels };
  },

  /**
   * Hands the server a UnifiedPush endpoint, so it can wake this phone when Shiver is not running.
   *
   * Through a plugin *action* rather than a write to the row, unlike the two above. The endpoint is
   * a url this server will later fetch, so it has to be checked before it is stored — a check that
   * only the server can do and that a page could not be trusted with. The action also takes the
   * caller's own user id from the session rather than from anything sent here, so this relay cannot
   * register a phone against somebody else's account even if the page asked it to.
   */
  setPushEndpoint: async (store, payload) => {
    const endpoint = typeof payload?.endpoint === 'string' ? payload.endpoint : '';

    if (!endpoint.startsWith('https://')) throw new Error('A push endpoint must be https');

    return store.actions.executePluginAction(PLUGIN_ID, 'setPushEndpoint', { endpoint });
  },

  /** Stops waking one device, or every device when no endpoint is named. */
  clearPushEndpoint: async (store, payload) =>
    store.actions.executePluginAction(PLUGIN_ID, 'clearPushEndpoint', {
      endpoint: typeof payload?.endpoint === 'string' ? payload.endpoint : undefined
    }),

  /**
   * The signed-in user's own status, and a way to change it.
   *
   * Here as well as in the settings screen because Shiver's bridge puts a button beside Sharkord's own
   * settings gear, which is a shorter trip than the settings screen for something people change
   * often. Both routes end at the same server action, which takes the caller's id from the session —
   * so neither can write a status onto anybody else.
   */
  getOwnStatus: async (store) => {
    const ownUserId = store.getState().ownUserId;
    const result = await store.actions.executePluginAction(PLUGIN_ID, 'getStatuses');
    const mine = (result?.statuses ?? []).find((entry) => entry?.userId === ownUserId);

    return { status: mine?.status ?? '' };
  },

  setStatus: async (store, payload) => {
    const result = await store.actions.executePluginAction(PLUGIN_ID, 'setStatus', {
      status: typeof payload?.status === 'string' ? payload.status : ''
    });

    // Written into this page's own map as well as sent, and that is the whole reason the button
    // works. The server tells everyone with `toAll`, but a push only reaches plugin components that
    // are *mounted*, and the member list is a sidebar Sharkord unmounts when it is closed. Set your
    // status from the button with that sidebar shut and there is nothing listening: the write lands
    // on the server and this page never hears about it. The settings screen never showed the
    // problem, because the component doing the saving was itself mounted.
    const ownUserId = store.getState().ownUserId;

    if (ownUserId !== undefined) applyStatus(ownUserId, result?.status ?? '');

    return { status: result?.status ?? '' };
  }
};

const respond = (id, body) => {
  window.postMessage({ source: RESPONSE, id, ...body }, window.location.origin);
};

const handle = async (event) => {
  // same-page only: not from an iframe, not from another window, not from another origin
  if (event.source !== window || event.origin !== window.location.origin) return;

  const data = event.data;

  if (!data || data.source !== REQUEST || typeof data.id !== 'string') return;

  const action = Object.hasOwn(ACTIONS, data.action) ? ACTIONS[data.action] : null;

  if (!action) {
    respond(data.id, { ok: false, error: `Unknown action: ${data.action}` });

    return;
  }

  const store = window.__SHARKORD_STORE__;

  if (!store) {
    respond(data.id, { ok: false, error: 'Sharkord store is not available' });

    return;
  }

  try {
    const result = await action(store, data.payload);

    respond(data.id, { ok: true, result });
  } catch (error) {
    respond(data.id, { ok: false, error: error?.message ?? String(error) });
  }
};

window.addEventListener('message', handle);

// how the bridge knows the plugin is installed here without asking the server
window.__SHIVER_PLUGIN__ = { version: 1 };

/* ─────────────────────────── custom statuses ─────────────────────────── */

/**
 * A line a user writes about themselves, shown beside their name.
 *
 * Rendered through Sharkord's own plugin slots rather than by Shiver's bridge, which is the whole
 * reason it works at all: a status is only useful if *other* people see it, and Shiver's client can
 * only change what its own user sees. Through the slots it reaches everyone on the server, in a
 * browser as much as in Shiver.
 *
 * `MEMBER_LIST_ITEM` shows it; `USER_SETTINGS` is where you write your own. Shiver's bridge adds a
 * button beside the settings gear that reaches the same action through the relay above — a
 * shortcut for Shiver's users, not a replacement for the field, which is the only route anyone in a
 * browser has.
 */

/**
 * How a status is drawn once it stops being short.
 *
 * Statuses are capped at 100 characters (`MAX_STATUS`, server side), and at one line in 12px that
 * is far more than a member row is wide — so the choice is truncate, wrap, or shrink. Truncating
 * was the old behaviour and hid most of a long status behind a tooltip nobody hovers.
 *
 * So: wrap, and step the size down as it gets longer, which buys back roughly a third of the
 * characters per line before any wrapping is needed at all. The steps are in px rather than
 * Tailwind classes because they are chosen against the *length of this string*, which no utility
 * class knows about.
 *
 * Clamped to three lines even so. A hundred characters at 10px will fit inside that in every place
 * this is used, so the clamp is a floor under the worst case rather than something anyone meets:
 * a member list where one person's row is five lines tall is a member list nobody can read.
 */
const statusStyle = (status) => ({
  fontSize: status.length <= 28 ? '12px' : status.length <= 56 ? '11px' : '10px',
  lineHeight: '1.3',
  whiteSpace: 'normal',
  // `anywhere` rather than `break-word`: a 100-character url with no spaces in it is one word, and
  // "do not break words" would put it back to overflowing the row it sits in
  overflowWrap: 'anywhere',
  display: '-webkit-box',
  WebkitBoxOrient: 'vertical',
  WebkitLineClamp: 3,
  overflow: 'hidden'
});

/** Everyone's status, so drawing a member list costs nothing. Kept in step by the server's push. */
const statuses = new Map();

/**
 * Listeners keyed by the user each one is drawing.
 *
 * One flat set, woken on every change, meant that one person updating their status re-rendered
 * every row in the member list — and each of those rows then scanned the whole user array to find
 * itself, so a 500-member server paid 250,000 comparisons on the main thread for one status. The
 * rows that care about a user are the only ones that need waking.
 */
const listeners = new Map();
let loaded = false;

const listenFor = (userId, listener) => {
  const held = listeners.get(userId) ?? new Set();

  held.add(listener);
  listeners.set(userId, held);

  return () => {
    held.delete(listener);

    if (!held.size) listeners.delete(userId);
  };
};

const announce = (userId) => {
  for (const listener of listeners.get(userId) ?? []) listener();
};

/** Every listening row, for the one case that genuinely changes all of them: a bulk load. */
const announceAll = () => {
  for (const held of listeners.values()) {
    for (const listener of held) listener();
  }
};

const applyStatus = (userId, status) => {
  if (!Number.isInteger(userId)) return;

  if (status) {
    statuses.set(userId, status);
  } else {
    statuses.delete(userId);
  }

  announce(userId);
};

/** Asked for once per page, not once per member row. */
const loadStatuses = async (store) => {
  if (loaded) return;

  loaded = true;

  try {
    const result = await store.actions.executePluginAction(PLUGIN_ID, 'getStatuses');

    for (const entry of result?.statuses ?? []) {
      if (!Number.isInteger(entry?.userId)) continue;

      if (entry.status) {
        statuses.set(entry.userId, entry.status);
      } else {
        statuses.delete(entry.userId);
      }
    }

    // one wake for the whole load rather than one per entry, which on a server with a hundred
    // statuses was a hundred renders of every row that happened to be mounted
    announceAll();
  } catch {
    // a server that refuses simply has no statuses to show; the rest of the plugin is unaffected
    loaded = false;
  }
};

/**
 * Subscribes a component to the shared map.
 *
 * One subscription per rendered row, and each is a set entry rather than a listener on the whole
 * store — the member list can be a hundred rows, and Sharkord's own `subscribe` fires on every state
 * change in the app.
 */
const useStatuses = (React, store, userId) => {
  const [, bump] = React.useState(0);

  React.useEffect(() => {
    const listener = () => bump((value) => value + 1);

    // Nothing was listening until now, so nothing has been heard since the last time something was.
    // The member list is a sidebar that unmounts when closed, and pushes that arrive while it is
    // shut are not delivered anywhere — so the map is stale by definition and asking again is the
    // only way to be right. Reopening the sidebar costs one query, not one per member.
    if (listeners.size === 0) loaded = false;

    const stop = listenFor(userId, listener);

    void loadStatuses(store);

    return stop;
  }, [store, userId]);

  // stable, so the host is not handed a new handler on every render of every member row
  const onPush = React.useCallback((data) => {
    if (data?.kind === 'status') applyStatus(data.userId, data.status);
  }, []);

  store.hooks.usePush(onPush);
};

/**
 * One user, by id, without walking the whole list to find them.
 *
 * `users.find(...)` per row per render is O(n) inside an O(n) render, so the member list was
 * quadratic in its own length. The index is rebuilt only when the array identity changes, which is
 * when Sharkord has actually replaced it.
 */
let userIndex = { source: null, byId: new Map() };

const userById = (store, userId) => {
  const users = store.getState().users;

  if (!users) return undefined;

  if (userIndex.source !== users) {
    userIndex = { source: users, byId: new Map(users.map((user) => [user.id, user])) };
  }

  return userIndex.byId.get(userId);
};

/** What the member list shows beside a name. Nothing at all when there is nothing to say. */
const MemberStatus = ({ userId }) => {
  const React = window.__SHARKORD_REACT__;
  const store = window.__SHARKORD_STORE__;

  useStatuses(React, store, userId);

  const status = statuses.get(userId);

  if (!status) return null;

  // Only while they are actually here. A line saying "back in ten" under someone who logged off
  // three days ago is worse than no line, and Sharkord already tracks presence for the sorting.
  const user = userById(store, userId);

  if (user?.status !== 'online') return null;

  // Held to the right and hard-capped, because the row it sits in is not ours.
  //
  // Sharkord's member row is `flex items-center gap-3`, and the name beside this has `truncate`
  // *without* `min-w-0` — so in flex terms the name cannot shrink below its content, and a long
  // status pushed the row instead of giving way. `ml-auto` parks this at the far end, `min-w-0`
  // lets it shrink at all, and `max-w-[45%]` stops it ever dominating the row.
  //
  // The width cap is what makes wrapping the right answer rather than a hazard: it can only ever
  // grow downwards, into a taller row, never sideways into the name. The title is still the whole
  // line, for the case where three clamped lines are not all of it.
  return React.createElement(
    'span',
    {
      className: 'ml-auto min-w-0 max-w-[45%] shrink text-muted-foreground',
      style: statusStyle(status),
      title: status
    },
    status
  );
};

/**
 * The same line in the profile card, through Sharkord's `user_popover` slot.
 *
 * The slot sits in the card's footer row, beside the direct-message and moderate buttons — which is
 * a row of icons, not a place for a sentence. So the line asks for a row of its own: `flex-basis`
 * of the whole width with `order: -1` puts it above the buttons, and the parent is told to wrap
 * through a rule keyed to this element being inside it.
 *
 * Written to degrade rather than break. If that rule ever stops applying — a Sharkord update, a
 * browser without `:has()` — the line simply stays on the buttons' row, where `truncate` and the
 * tooltip keep it readable. The worst case is a cramped status, not a broken card.
 */
const POPOVER_STATUS_CLASS = 'shiver-popover-status';
const POPOVER_STATUS_STYLE_ID = 'shiver-popover-status-style';

const ensurePopoverStatusStyle = () => {
  if (document.getElementById(POPOVER_STATUS_STYLE_ID)) return;

  const style = document.createElement('style');

  style.id = POPOVER_STATUS_STYLE_ID;
  style.textContent = `
.${POPOVER_STATUS_CLASS} { flex-basis: 100%; min-width: 0; order: -1; text-align: left; }

/* The row the slot renders into, which is sized by its contents. Without the cap a long status
   makes that row as wide as itself and the card grows past its own edge — measured, not guessed.
   The cap gives the truncation something to bite on and leaves the "member since" line its half.

   Qualified with a tag name. A bare :has(> ...) has no left-hand side at all, so the browser
   evaluates it against every element in the document on every style recalculation — for the whole
   Sharkord client, not just the one card this was written for. */
div:has(> .${POPOVER_STATUS_CLASS}) { flex-wrap: wrap; max-width: 65%; }
`;

  document.head.append(style);
};

const PopoverStatus = ({ userId }) => {
  const React = window.__SHARKORD_REACT__;
  const store = window.__SHARKORD_STORE__;

  useStatuses(React, store, userId);

  React.useEffect(ensurePopoverStatusStyle, []);

  const status = statuses.get(userId);

  if (!status) return null;

  // Same rule as the member list: only while they are actually here. A card is opened deliberately
  // about one person, which makes a stale line more misleading here, not less.
  const user = userById(store, userId);

  if (user?.status !== 'online') return null;

  return React.createElement(
    'span',
    {
      className: `${POPOVER_STATUS_CLASS} text-muted-foreground`,
      style: statusStyle(status),
      title: status
    },
    status
  );
};

/**
 * Where you write your own, in Sharkord's own user settings.
 *
 * Shiver's bridge puts a button beside Sharkord's settings gear that does the same thing in one
 * click, and this field was briefly removed as a duplicate of it. It is back because the two are
 * not duplicates: the button is drawn by Shiver and exists only inside Shiver, so without this field a
 * browser — and anyone else on the server not using Shiver — can read everyone's status and never
 * write their own. Both routes end at the same server action, which takes the caller's id from the
 * session, so neither can write a status onto anybody else.
 */
const StatusSetting = () => {
  const React = window.__SHARKORD_REACT__;
  const store = window.__SHARKORD_STORE__;

  const ownUserId = store.getState().ownUserId;

  useStatuses(React, store, ownUserId);

  const [draft, setDraft] = React.useState('');
  const [saving, setSaving] = React.useState(false);
  const [started, setStarted] = React.useState(false);

  // seeded once from what the server already has, then left alone so typing is not fought with
  React.useEffect(() => {
    if (started || ownUserId === undefined) return;

    setDraft(statuses.get(ownUserId) ?? '');
    setStarted(true);
  }, [ownUserId, started]);

  const save = async () => {
    setSaving(true);

    try {
      const result = await store.actions.executePluginAction(PLUGIN_ID, 'setStatus', {
        status: draft
      });

      if (ownUserId !== undefined) applyStatus(ownUserId, result?.status ?? '');
    } catch {
      // left as typed, so the attempt is not silently lost
    } finally {
      setSaving(false);
    }
  };

  return React.createElement(
    'div',
    { className: 'flex flex-col gap-2' },
    React.createElement(
      'label',
      { className: 'text-sm font-medium', htmlFor: 'shiver-status' },
      'Status'
    ),
    React.createElement(
      'p',
      { className: 'text-xs text-muted-foreground' },
      'A line shown beside your name while you are online. Everyone on this server can see it.'
    ),
    React.createElement(
      'div',
      { className: 'flex items-center gap-2' },
      React.createElement('input', {
        id: 'shiver-status',
        className:
          'flex-1 rounded border border-input bg-transparent px-2 py-1.5 text-sm outline-none',
        value: draft,
        maxLength: 100,
        placeholder: 'What are you up to?',
        onChange: (event) => setDraft(event.target.value)
      }),
      React.createElement(
        'button',
        {
          type: 'button',
          className: 'rounded bg-primary px-3 py-1.5 text-sm text-primary-foreground',
          disabled: saving,
          onClick: () => void save()
        },
        saving ? 'Saving…' : 'Save'
      )
    )
  );
};

// The slot ids are Sharkord's own, given as strings so this bundle imports nothing and needs no
// build step — the enum values are `member_list_item`, `user_popover` and `user_settings`.
export const components = {
  member_list_item: [MemberStatus],
  user_popover: [PopoverStatus],
  user_settings: [StatusSetting]
};
