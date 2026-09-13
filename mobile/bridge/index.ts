/**
 * The Shiver bridge, mobile.
 *
 * Runs inside a Sharkord page, like its desktop counterpart, and under nearly the same rule: it is
 * handed this server's own data, plus the least about the others that will draw a rail — a name, a
 * logo as bytes, an opaque id. No other server's *address* is reachable from here. See
 * `rail_payload` in `webview.rs` for why that line is where it is.
 *
 * It differs from the desktop bridge in when it runs and what it is for. On desktop the bridge is
 * an initialization script that *must* execute before the page's own scripts, because it seeds the
 * session that stops the login page appearing. There is no session to seed here — `keyring` has no
 * Android backend, so Shiver holds no credentials and Sharkord's own auto-login does that job from
 * the webview's storage. So this is evaluated after the page has loaded, which is also why it must
 * be safe to run twice: a reload or an in-app navigation brings it back.
 *
 * What it is for is the navigation Android takes away. One webview per window means Shiver has no
 * chrome of its own while a server is up, so the server rail has to be drawn in the server's page.
 * Sharkord's own mobile layout already answers a swipe right with its channel drawer; Shiver adds the
 * level above it, so the gesture reads the same way down as on desktop: content, channels, servers.
 */

type ShiverTheme = {
  themeColor: string;
  /** the text colour the user picked, or null to take it from the background */
  textColor: string | null;
  accentColor: string;
};

/** One server, as much of it as a server's page is allowed to know. Deliberately no origin. */
type RailEntry = {
  id: string;
  name: string;
  /** the logo as a `data:` uri, or null to fall back to initials */
  icon: string | null;
  /** unread on that server, counted by the core over its own connection */
  unread: number;
  /**
   * Shiver's session for this server expired and it has no way to renew one.
   *
   * Shown, because the alternative is what happened before: the server went quiet — no badge, no
   * messages, nothing in the direct-message list — and nothing said why.
   */
  signedOut: boolean;
  /** where it sits among the rail's top level, or among its folder's contents */
  position: number;
  /** the folder holding it, or null when it sits at the top level */
  folderId: string | null;
};

/** A folder in the rail. Shiver's own furniture: a name the user typed and a place in the order. */
type RailFolder = {
  id: string;
  name: string;
  position: number;
  expanded: boolean;
};

/**
 * One conversation on *this* server, read from this page's own Sharkord store.
 *
 * Never handed over by the core. Shiver knows the user's conversations on every server they have
 * added, and putting that list in a page would tell one server who the user privately messages on
 * all the others — so the cross-server list lives on Shiver's own screen, which a server's page
 * cannot read. What is built here is this page's own data, which it already had.
 */
type DirectMessage = {
  entryId: string;
  serverName: string;
  channelId: number;
  userName: string;
};

type ShiverConfig = {
  entryId: string;
  origin: string;
  serverName: string;
  /** absent while the user is on Sharkord's own colours, and then nothing is restyled */
  theme: ShiverTheme | null;
  muted: number[];
  /** where Shiver's own pages live; the rail navigates there rather than calling, having no IPC */
  home: string;
  rail: RailEntry[];
  folders: RailFolder[];
  /** the user arrived by tapping the rail's direct-messages tile */
  openDms: boolean;
  /** the conversation to land on, named by the person it is with, when one was tapped */
  openDmUser: string | null;
  /** this server's own session, when Shiver signed in for the user; never any other server's */
  session: string | null;
  /**
   * The UnifiedPush endpoint for *this* server, when Shiver has one.
   *
   * Handed to the companion plugin so this server can wake the phone while Shiver is closed. It is
   * this entry's alone — Shiver registers one instance per server precisely so a page never sees a
   * capability belonging to another.
   */
  pushEndpoint: string | null;
  /** the settings Shiver kept when it last wiped this server's storage, as json */
  carried: string | null;
  /**
   * Shrink the file card under a picture down to its icon.
   *
   * Sharkord draws an image attachment twice — the picture, and a card below it with the filename
   * and the size. This is whether the second one is worth the room it takes, which on a phone is
   * a sharper question than on a desktop.
   */
  minimiseAttachments: boolean;
  /**
   * How loud this server's own sounds should be, as a percentage. 100 is Sharkord's own level.
   *
   * Above 100 is the point of it: an `<audio>` element's volume stops at 1.0, but Sharkord
   * synthesises every sound it makes through Web Audio, and a gain node has no such ceiling.
   */
  soundVolume: number;
};

declare global {
  interface Window {
    __SHIVER__?: ShiverConfig;
    __SHIVER_MOBILE_INSTALLED__?: boolean;
    /** the rail this document is currently showing, so listeners bound once can find the live one */
    __SHIVER_RAIL__?: Rail;
    /** called by the core when a badge changes; this page cannot listen for events itself */
    __SHIVER_UNREAD__?: (unread: Record<string, number>) => void;
    /** called by the core for the two menu actions only this server's own client can perform */
    __SHIVER_MARK_ALL_READ__?: () => void;
    __SHIVER_SIGN_OUT__?: () => void;
    /** changes how loud this page's own sounds are, without reloading it */
    __SHIVER_SET_SOUND_VOLUME__?: (percent: number) => void;
    /** turns the minimised attachment cards on or off without reloading the page */
    __SHIVER_SET_ATTACHMENT_CARDS__?: (minimised: boolean) => void;
    /** called by the core just before it navigates away from this server */
    __SHIVER_FORGET_SESSION__?: () => void;
    /** set only when Shiver put this page's sign-in there, so it only clears up after itself */
    __SHIVER_SEEDED__?: boolean;
    /** the companion plugin's relay, present only on a server that has it installed */
    __SHIVER_PLUGIN__?: { version: number };
    /**
     * This server's muted channels, as the page currently knows them.
     *
     * Read by the core on its way out of this server, which is the moment it starts counting that
     * server's unread itself and so the moment it needs to know what to leave out. Reached through
     * `window` rather than a closure because the listeners below are bound once per document while
     * the mute set is rebuilt on every install.
     */
    __SHIVER_MUTED__?: () => number[];
    /** flips one channel's mute, for the long-press menu bound once per document */
    __SHIVER_TOGGLE_MUTE__?: (channelId: number) => void;
    /** the phone's back button, answered by the rail; true when Shiver used the press */
    __SHIVER_BACK__?: () => boolean;
    /**
     * What the rail has become: its top level in order, and any server moved between folders.
     *
     * Read by the core on a timer. Moves are handed over once and then forgotten here, so a move
     * that has been stored is not asked for again on the next look.
     */
    /** addresses the page asked to open elsewhere, handed over and cleared by the core's poll */
    __SHIVER_OPEN__?: () => string[];
    __SHIVER_RAIL_STATE__?: () => {
      order: { kind: 'server' | 'folder'; id: string }[];
      moves: { serverId: string; folderId: string | null }[];
      creates: { id: string; name: string; memberIds: string[] }[];
    };
  }
}

const ROOT_ID = 'shiver-root';
const MUTE_CLASS = 'shiver-muted-channel';
const MUTE_STYLE_ID = 'shiver-mute-style';
const QUIET_STYLE_ID = 'shiver-quiet-style';
const MENU_HOST_ID = 'shiver-channel-menu';
const SHIVER_MENU_ITEM = 'shiver-menu-item';
/**
 * What Shiver handed this page, kept here rather than on `window`.
 *
 * It arrives as `window.__SHIVER__` — an inlined global is the only way in, the page having no IPC —
 * and it is moved here and the global deleted the moment it is read. It carries this server's
 * session token, which is this origin's own, so leaving it there was not a cross-server leak; it
 * was a wider blast radius, any script the server loads being able to read the token off a global
 * instead of reaching into storage.
 *
 * Assigned at the very bottom of this file, with `install`, so anything reading it is called after.
 * The window here is wider than desktop's: the bridge runs after page load rather than before the
 * page's own scripts (see the module note in `webview.rs`), so the global exists for as long as the
 * page took to load. Much smaller than forever, and not closable from this end.
 *
 * One consequence worth knowing. A second install in the same document runs as its own copy of this
 * file, so the listeners bound once by the first — the gestures, the reconnect overlay, the channel
 * menu — keep reading the first copy's config rather than the newer one. That was not true while
 * they read `window.__SHIVER__`, which a later install overwrote. It costs nothing here because all
 * three things they read are constant for a document: `home` is fixed for the run, `serverName` is
 * this page's own server, and a re-seeded `session` arrives with a rebuilt page and so a new
 * document. Anything added here that can change *within* a document needs the treatment the rail
 * gets — reached through `window`, not closed over.
 */
let shiverConfig: ShiverConfig | null = null;

const MESSAGE_ACTIONS_CLASS = 'shiver-message-actions';
const ACTIONS_STYLE_ID = 'shiver-actions-style';
const RECONNECT_HOST_ID = 'shiver-reconnect';
const THEME_STYLE_ID = 'shiver-theme';

/** Sharkord's own markers. Its rows carry these; they are the seam Shiver works against. */
const SIDEBAR = '[data-testid="left-sidebar"]';
const DM_TOGGLE = '[data-testid="dm-toggle"]';
const CHANNEL_ITEM = '[data-testid="channel-item"]';
const DM_ITEM = '[data-testid="dm-item"]';
const UNREAD_COUNT = '[data-testid="unread-count"]';
const MESSAGE_ITEM = '[data-testid="message-item"]';
const MEMBER_ITEM = '[data-testid="member-item"]';
/** Every message's wrapper carries its own id, which is the way up to the name above it. */
const MESSAGE_WRAPPER = '[id^="message-"]';
/**
 * The author's name in an inline reply's preview line.
 *
 * No test id on it, so it is matched by the three classes it carries together — and nothing else in
 * the client carries all three. Written as attribute selectors rather than `.max-w-40`, because a
 * class selector would need the escaping that has bitten here before.
 */
const REPLY_AUTHOR = '[class~="max-w-40"][class~="truncate"][class~="font-medium"]';
/**
 * A mention inside a message: `<span class="mention …">@Name</span>`.
 *
 * `mention` is Sharkord's own class, not a Tailwind one, so it is stable. The chip's text carries a
 * leading `@` that has to come off before the name can be matched.
 */
const MENTION_CHIP = 'span.mention';
/**
 * A reaction pill you are part of. Sharkord marks it with a one-pixel border and nothing else —
 * `border-border` when you have reacted, `border-none` when you have not — easy to miss on a phone.
 */
const REACTED_PILL = '[class~="h-9"][class~="border-border"]';

const RAIL_WIDTH = 68;
/** Sharkord's own threshold, so the second swipe feels like the first. */
const SWIPE_THRESHOLD = 50;
/** Tailwind's `md`, the width at which Sharkord stops hiding its sidebar. */
const WIDE_LAYOUT = 768;

function install(shiver: ShiverConfig) {
  // evaluated after every page load, so everything below has to tolerate already having run
  applyTheme(shiver.theme);

  // the rail is rebuilt on each install, because the servers in it may have changed
  window.__SHIVER_RAIL__ = mountRail(shiver);

  if (shiver.openDmUser) {
    // arrived by tapping a conversation in Shiver's own list; land on it
    openConversation(shiver.openDmUser);
  } else if (shiver.openDms) {
    // the user tapped the rail's dm tile on another screen; this is where that lands
    openDirectMessages();
  }

  installSoundVolume(shiver.soundVolume);
  installAttachmentCards(shiver.minimiseAttachments);
  installVoiceColors();
  installPageActions();
  quietTheLoadingScreen();
  styleMessageActions();

  if (shiver.carried) restoreCarried(shiver.carried);
  if (shiver.session) seedSession(shiver.session);

  // The set Shiver starts from is its own, and the server's copy is merged into it below. Held here
  // and reached through `window`, so the observer bound once per document always paints the current
  // set rather than the one the first install closed over.
  const muted = new Set(shiver.muted);

  const paint = () => paintMuted(new Set(window.__SHIVER_MUTED__?.() ?? []));

  window.__SHIVER_MUTED__ = () => [...muted];

  window.__SHIVER_TOGGLE_MUTE__ = (channelId) => {
    if (muted.has(channelId)) {
      muted.delete(channelId);
    } else {
      muted.add(channelId);
    }

    paint();

    // straight to the server, so the mute is on the user's other devices before they get there
    pushMutesToPlugin([...muted]);
  };

  paint();

  // The server's copy is what makes a mute follow the user between devices. It arrives late — the
  // client imports plugin bundles after connecting — so this settles well after the first paint.
  if (shiver.pushEndpoint) {
    registerPushWithPlugin(shiver.pushEndpoint).catch(() => undefined);
  }

  syncMutesWithPlugin([...muted]).then((merged) => {
    if (!merged) return;

    muted.clear();

    for (const id of merged) muted.add(id);

    paint();
  });

  // Bound once per document, and reaching the rail through `window` rather than closing over it.
  // A second install in the same document would otherwise leave the first set of listeners driving
  // a rail that is no longer on screen, and both would fight over the class on `<html>`.
  if (!window.__SHIVER_MOBILE_INSTALLED__) {
    installGestures(() => window.__SHIVER_RAIL__);
    installChannelMenu();
    installAutoReconnect();
    installRoleColors();
    installExternalLinks();
    installStatusButton();

    // Sharkord re-renders its channel list on scroll, selection and unread changes, each of which
    // drops the dimming, so it is reapplied whenever the sidebar's dom changes
    new MutationObserver(paint).observe(document.body, { childList: true, subtree: true });
    window.__SHIVER_MOBILE_INSTALLED__ = true;
  }
}

/**
 * Follows the links Android will not.
 *
 * Sharkord opens files and outside links with `target="_blank"` — an attachment card, a link in a
 * message, an open-graph preview. That is a request for a *new window*, not a navigation, so none
 * of it ever reached Shiver's navigation guard; and Android's webview drops such a request unless the
 * host implements `onCreateWindow`, which wry does not. The result was a tap that did nothing at
 * all, silently, for every file and every link in a message.
 *
 * Shiver has no second window to open and would not want one. The address is left here instead and
 * the core hands it to the browser on its next poll — the same place every other link out of
 * Sharkord goes, and the thing that knows how to download a file.
 *
 * Only `http(s)`, and only where the page meant to leave: a plain in-page link is still the page's
 * own business.
 */
const toLevel = (percent: number) => (Number.isFinite(percent) ? Math.max(0, percent) / 100 : 1);

/**
 * Routes every sound this page makes through one gain node, so the user's volume setting can take
 * it past what the page itself would ever play.
 *
 * Above 100% is the whole point, and it is only possible because of how both programs make sound:
 * Sharkord synthesises its tones through Web Audio — an oscillator into a gain into the context's
 * destination — and so does Shiver. A media element's `volume` is capped at 1.0; a `GainNode` is not.
 *
 * **Voice is untouched, and not by luck.** Sharkord plays the people in a call through media
 * elements, and the only `AudioContext` its call code builds is an analyser for the speaking
 * indicator, which is connected to nothing. Nothing a call makes reaches `destination`, so nothing
 * a call makes passes through here — the slider moves the notification tones and the join, leave,
 * mute and camera clicks, and leaves how loud people are alone.
 *
 * Android's own volume keys move everything the phone plays at once; this moves the ping relative
 * to the call, which is the part the system volume cannot do.
 */
function installSoundVolume(percent: number) {
  // Patching `connect` twice would nest one master inside another and square the level, so a
  // second install changes the number rather than the wiring.
  const installed = window.__SHIVER_SET_SOUND_VOLUME__;

  if (installed) {
    installed(percent);

    return;
  }

  // a config from before this setting existed carries no percentage, and NaN in a gain throws
  let level = toLevel(percent);

  const masters = new Set<GainNode>();
  const perContext = new WeakMap<BaseAudioContext, GainNode>();

  // narrowed to the node-to-node overload for the same reason `silenceMessagePing` narrows it:
  // `connect` is also declared for AudioParam, and `.call` otherwise resolves to that one
  const connect = AudioNode.prototype.connect as (
    this: AudioNode,
    destination: AudioNode,
    output?: number,
    input?: number
  ) => AudioNode;

  // one per context, made on demand — a page that never makes a sound never builds one
  const masterFor = (context: BaseAudioContext) => {
    const existing = perContext.get(context);

    if (existing) return existing;

    const gain = context.createGain();

    gain.gain.value = level;
    connect.call(gain, context.destination);

    perContext.set(context, gain);
    masters.add(gain);

    return gain;
  };

  AudioNode.prototype.connect = function patched(
    this: AudioNode,
    destination: AudioNode | AudioParam,
    output?: number,
    input?: number
  ) {
    if (destination instanceof AudioDestinationNode) {
      // the output index is kept, the input index is not: the master has exactly one input, and
      // the page's own index would be out of range on it
      return connect.call(this, masterFor(destination.context), output);
    }

    return connect.call(this, destination as AudioNode, output, input);
  } as AudioNode['connect'];

  window.__SHIVER_SET_SOUND_VOLUME__ = (next: number) => {
    level = toLevel(next);

    for (const gain of masters) {
      gain.gain.value = level;
    }
  };
}

function installExternalLinks() {
  const queued: string[] = [];

  window.__SHIVER_OPEN__ = () => queued.splice(0, queued.length);

  document.addEventListener(
    'click',
    (event) => {
      if (!event.isTrusted || event.defaultPrevented) return;

      const anchor = (event.target as Element | null)?.closest?.('a');

      if (!(anchor instanceof HTMLAnchorElement)) return;
      // a link that opens in place is a navigation, and the guard already handles those
      if (anchor.target !== '_blank') return;

      // A control *inside* the link is not the link.
      //
      // Sharkord's attachment card is an `<a target="_blank">` to the file with a delete button
      // sitting inside it, and that button's own handler cancels the anchor. It never got the
      // chance: this listener captures, so it ran first, called `stopPropagation`, and the tap
      // never reached React at all — delete handed the attachment to the browser instead of
      // deleting it. Reported on desktop, fixed in both, because it is the same page either way.
      const control = (event.target as Element | null)?.closest?.(
        'button, input, select, textarea, label, [role="button"], [contenteditable]'
      );

      if (control && control !== anchor && anchor.contains(control)) return;

      const href = anchor.href;

      if (!/^https?:/i.test(href)) return;

      event.preventDefault();
      event.stopPropagation();

      queued.push(href);
    },
    true
  );
}

/* ─────────────────────────── role colours ─────────────────────────── */

/**
 * Draws each username in the colour of their highest role.
 *
 * Sharkord gives roles a colour and then never uses it on a name — it shows up on the role's own
 * badge and nowhere else — so a server's colour scheme is invisible in the two places people
 * actually read names. This puts it there.
 *
 * **What "highest" means here.** Sharkord has no role hierarchy: roles carry no position, and the
 * server hands them back in the order they were made. So Shiver says it plainly — the first role in
 * the server's own list that the user holds, which is the topmost one in Settings → Roles, with
 * two rules on top:
 *
 * - the default role is the floor, used only when the user has nothing else. It is the role
 *   everybody has, so letting it win would paint the whole server one colour and hide every role
 *   that was given a colour on purpose.
 * - `#ffffff` is read as "no colour chosen", because that is the column's default in Sharkord's
 *   schema and a name is near enough white already. A role left alone is skipped rather than
 *   drawn.
 *
 * Names are matched by their text, because that is all the dom has: Sharkord puts no user id on a
 * message or a member row. Two people with the same display name would be coloured alike, which is
 * the same limitation the muted-channel dimming already lives with.
 */
function installRoleColors() {
  let colors = new Map<string, string>();
  let ownName = '';
  let signature = '';

  /** Which of a user's roles gives them their colour. See the note above for what decides it. */
  const colorFor = (user: SharkordUser, roles: SharkordRole[]) => {
    const held = roles.filter((role) => user.roleIds?.includes(role.id));
    const chosen = (role: SharkordRole) => {
      const color = role.color?.trim().toLowerCase();

      return color && color !== '#ffffff' && color !== '#fff' ? role.color : null;
    };

    for (const role of held) {
      if (role.isDefault) continue;

      const color = chosen(role);

      if (color) return color;
    }

    // nothing but the role everyone has, so its colour is the one they were given
    for (const role of held) {
      if (!role.isDefault) continue;

      const color = chosen(role);

      if (color) return color;
    }

    return null;
  };

  const read = () => {
    const state = sharkordStore()?.getState();

    if (!state?.users || !state.roles) return false;

    const next = new Map<string, string>();

    for (const user of state.users) {
      const color = colorFor(user, state.roles);

      if (color) next.set(user.name, color);
    }

    const stamp = [...next].map(([name, color]) => `${name}:${color}`).join();

    if (stamp === signature) return false;

    signature = stamp;
    colors = next;
    // kept so a mention of yourself can be left as Sharkord painted it
    ownName = state.users.find((user) => user.id === state.ownUserId)?.name ?? '';

    return true;
  };

  /** The name a node is showing, when it is not simply the node's own text. */
  const applyNamed = (node: HTMLElement, name: string) => {
    const color = colors.get(name);

    if (color) {
      node.style.color = color;
      node.dataset.shiverRole = '1';

      return;
    }

    if (node.dataset.shiverRole) {
      node.style.color = '';

      delete node.dataset.shiverRole;
    }
  };

  const apply = (node: HTMLElement | null | undefined) => {
    if (!node) return;

    const color = colors.get(node.textContent?.trim() ?? '');

    if (color) {
      node.style.color = color;
      node.dataset.shiverRole = '1';

      return;
    }

    // only ever undone where Shiver put it, so a name Sharkord colours itself is left alone
    if (node.dataset.shiverRole) {
      node.style.color = '';

      delete node.dataset.shiverRole;
    }
  };

  const paint = () => {
    // The name sits above the messages rather than inside one: each message wrapper carries an id,
    // its parent is the column of messages, and the header beside that column holds the name. Read
    // that way round because the wrapper's id is the only stable hook in the group.
    for (const wrapper of document.querySelectorAll<HTMLElement>(MESSAGE_WRAPPER)) {
      const header = wrapper.parentElement?.previousElementSibling;

      apply(header?.querySelector<HTMLElement>(':scope > span'));
    }

    for (const row of document.querySelectorAll<HTMLElement>(MEMBER_ITEM)) {
      apply(row.querySelector<HTMLElement>(':scope > span'));
    }

    // The name in a reply's preview line is the same person, and was the one place their colour
    // was dropped — a reply to somebody quoted them in the ordinary grey.
    for (const name of document.querySelectorAll<HTMLElement>(REPLY_AUTHOR)) {
      apply(name);
    }

    // A mention is the same person again. Your own is left alone: Sharkord paints it yellow to say
    // *you* were pinged, which is a different thing from who somebody is.
    for (const chip of document.querySelectorAll<HTMLElement>(MENTION_CHIP)) {
      const name = chip.textContent?.trim().replace(/^@/, '') ?? '';

      if (!name || name === ownName) continue;

      applyNamed(chip, name);
    }
  };

  const refresh = () => {
    read();
    paint();
  };

  const start = () => {
    const store = sharkordStore();

    if (!store?.subscribe) return false;

    store.subscribe(refresh);
    refresh();

    return true;
  };

  // The store arrives with the client rather than with the page, so this waits for it — the same
  // wait the desktop bridge does, for the same reason.
  if (!start()) {
    const timer = window.setInterval(() => {
      if (start()) window.clearInterval(timer);
    }, 500);
  }

  // Messages and members are drawn as they arrive and redrawn as the user scrolls, and neither
  // touches the store, so the dom is what has to be watched for the rest.
  new MutationObserver(paint).observe(document.body, { childList: true, subtree: true });
}

/* ────────────────────────────── the rail ────────────────────────────── */

type Rail = {
  open: () => void;
  close: () => void;
  isOpen: () => boolean;
};

/**
 * Draws Shiver's rail over the server's client.
 *
 * In a closed shadow root, which is not decoration: Sharkord ships Tailwind's preflight and Shiver's
 * own markup would be restyled by it, while Shiver's rules would leak the other way into a client
 * Shiver's whole purpose is not to reimplement. The one thing that has to reach out is the offset
 * that slides Sharkord's drawer aside, and that is a single class on `<html>`.
 */
function mountRail(shiver: ShiverConfig): Rail {
  document.getElementById(ROOT_ID)?.remove();

  const host = document.createElement('div');

  host.id = ROOT_ID;
  document.body.appendChild(host);

  const root = host.attachShadow({ mode: 'closed' });

  root.appendChild(railStyle());

  const scrim = el('div', 'scrim');
  const nav = el('nav', 'rail');

  nav.setAttribute('aria-label', 'Servers');

  let open = false;

  // The rail slides *over* Sharkord's drawer and moves nothing but itself.
  //
  // It used to push the drawer aside, so the channel names stayed readable — and that meant two
  // elements animating together, which they never quite did. A gap kept opening down the left edge
  // on the way out, because the transform that pushed the drawer was scoped to a class: the moment
  // the rail was dismissed the rule vanished and the drawer returned under its own timing, however
  // carefully Shiver matched the two. Covering the drawer instead removes the second animation
  // entirely, and with it every way the two can disagree. It also puts Shiver back to touching none
  // of the server's own layout, which is the rule the rest of this file keeps.
  const setOpen = (next: boolean) => {
    open = next;

    // Anything the rail opened goes with it. The direct-message list is drawn beside the rail
    // rather than inside it, so it does not slide away on its own — closed by a swipe it simply
    // stayed on screen with nothing left to dismiss it.
    if (!next) {
      root.querySelector('.dm-panel')?.remove();
      root.querySelector('.menu')?.remove();
    }

    host.classList.toggle('open', next);
  };

  /**
   * What the phone's back button does here, innermost thing first.
   *
   * Returns whether Shiver used the press, so the activity can fall back to what it would have done
   * otherwise on a page Shiver is not drawing on.
   */
  window.__SHIVER_BACK__ = () => {
    if (menuHost) {
      closeChannelMenu();

      return true;
    }

    const menu = root.querySelector('.menu');

    if (menu) {
      menu.remove();

      return true;
    }

    const panel = root.querySelector('.dm-panel');

    if (panel) {
      panel.remove();

      return true;
    }

    setOpen(!open);

    return true;
  };

  scrim.addEventListener('click', () => setOpen(false));

  // ── the tiles, in the desktop rail's order ──

  nav.append(
    tile({
      label: 'Direct messages',
      content: messagesIcon(),
      onPick: () => {
        // a second tap on the tile puts it away again, the way the tile itself would be expected to
        if (root.querySelector('.dm-panel')) {
          root.querySelector('.dm-panel')?.remove();

          return;
        }

        openDmPanel(root, shiver, () => setOpen(false));
      }
    }),
    el('div', 'divider')
  );

  const badges = new Map<string, HTMLElement>();

  /**
   * The rail's top level: folders and loose servers together, in the order the user put them in.
   *
   * Servers inside a folder are not part of this ordering — they are drawn under their folder when
   * it is open, and keep their own order relative to each other.
   */
  const openFolders = new Set(
    shiver.folders.filter((folder) => folder.expanded).map((folder) => folder.id)
  );

  const membersOf = (folderId: string) =>
    shiver.rail
      .filter((entry) => entry.folderId === folderId)
      .sort((a, b) => a.position - b.position);

  /**
   * Worked out on every draw, not once.
   *
   * Held as a constant it went stale the moment a server was moved between folders: the list still
   * had it at the top level while the folder now claimed it too, and the rail drew the same server
   * twice.
   */
  /**
   * Drops any folder that is no longer holding a group, which is the rule Shiver keeps too.
   *
   * Stated twice on purpose. Shiver is where it is decided — the store is what survives — but the
   * rail cannot wait for the store to answer, because there is no way for Shiver to talk back into a
   * server's page. Applying it here as well is what makes dragging the second-to-last server out
   * of a folder look like it did something, instead of leaving the folder on screen until the next
   * time the page loads.
   */
  const dissolveThinFolders = () => {
    const doomed = shiver.folders.filter((folder) => membersOf(folder.id).length < 2);

    if (!doomed.length) return;

    for (const folder of doomed) {
      for (const entry of shiver.rail) {
        if (entry.folderId === folder.id) entry.folderId = null;
      }

      openFolders.delete(folder.id);
    }

    shiver.folders = shiver.folders.filter((folder) => !doomed.includes(folder));
  };

  const topItems = (): (
    | { kind: 'folder'; folder: RailFolder }
    | { kind: 'server'; entry: RailEntry }
  )[] =>
    [
      ...shiver.folders.map((folder) => ({
        kind: 'folder' as const,
        folder,
        position: folder.position
      })),
      ...shiver.rail
        .filter((entry) => !entry.folderId)
        .map((entry) => ({ kind: 'server' as const, entry, position: entry.position }))
    ]
      .sort((a, b) => a.position - b.position)
      .map(({ position, ...item }) => item);

  const serverTile = (entry: RailEntry, inFolder: boolean) => {
    // marked before anything else, so a server waiting to be signed in reads differently from one
    // that simply has nothing new
    const asleep = entry.signedOut;
    const active = entry.id === shiver.entryId;
    const badge = el('span', 'badge');

    badge.hidden = true;
    badges.set(entry.id, badge);

    const button = tile({
      label: entry.name,
      active,
      badge,
      reorderable: true,
      className: [
        'tile-server',
        inFolder ? 'in-folder' : '',
        asleep ? 'signed-out' : ''
      ]
        .filter(Boolean)
        .join(' '),
      content: entry.icon ? image(entry.icon) : text(initials(entry.name)),
      onHold: (at) =>
        openServerMenu(root, entry, active, at, () => setOpen(false), shiver.folders, (folderId) => {
          // Recorded for the core, and applied here at once so the rail moves when the user says
          // so rather than a second later when the core next looks.
          pendingMoves.push({ serverId: entry.id, folderId });
          entry.folderId = folderId;

          if (folderId) openFolders.add(folderId);

          rebuild();
        }),
        onPick: () => {
          // already here: the rail is a switcher, and switching to where you are should not
          // reload the client you are looking at
          if (active) {
            setOpen(false);

            return;
          }

          goHome(`#open=${encodeURIComponent(entry.id)}`);
        }
    });

    button.dataset.entryId = entry.id;

    return button;
  };

  const addTile = tile({
    label: 'Add a server',
    className: 'add',
    content: plusIcon(),
    onPick: () => goHome('#add')
  });

  /**
   * Draws the servers and folders, and redraws them when a folder is opened or shut.
   *
   * Only this stretch of the rail changes: the tiles above and below it are fixtures, so they are
   * built once and the items are put back in between them.
   */
  const rebuild = () => {
    dissolveThinFolders();

    for (const stale of nav.querySelectorAll('.tile-server, .tile-folder')) stale.remove();

    badges.clear();

    for (const item of topItems()) {
      if (item.kind === 'server') {
        nav.insertBefore(serverTile(item.entry, false), addTile);

        continue;
      }

      const folder = item.folder;
      const members = membersOf(folder.id);
      const isOpen = openFolders.has(folder.id);
      const button = tile({
        label: folder.name,
        className: isOpen ? 'tile-folder open' : 'tile-folder',
        content: folderIcon(members, isOpen),
        reorderable: true,
        onHold: (at) => openFolderMenu(root, folder, at),
        onPick: () => {
          // Open and shut here only. Whether a folder is left open is Shiver's to remember, and this
          // page has no way to tell it, so the state lives as long as the rail does — the stored
          // one is whatever was last set on Shiver's own screen.
          if (openFolders.has(folder.id)) openFolders.delete(folder.id);
          else openFolders.add(folder.id);

          rebuild();
        }
      });

      button.dataset.folderId = folder.id;
      nav.insertBefore(button, addTile);

      if (!isOpen) continue;

      // the members sit in a box of their own, so an open folder reads as a group rather than as
      // more of the rail
      const contents = el('div', 'folder-contents');

      contents.dataset.folderId = folder.id;

      for (const entry of members) contents.append(serverTile(entry, true));

      nav.insertBefore(contents, addTile);
    }

    paintBadgesNow();
  };

  nav.append(
    addTile,
    el('div', 'spacer'),
    el('div', 'divider'),
    tile({
      label: 'Shiver settings',
      content: settingsIcon(),
      onPick: () => goHome('#settings')
    })
  );

  installRailDrag(nav, shiver, openFolders, rebuild);

  root.append(scrim, nav);

  /**
   * Repaints the badges.
   *
   * The core calls this rather than the page listening for an event: a page on a server's origin
   * has no Tauri IPC, so Shiver talks to it in the one direction that works — inward, the same way
   * the bridge itself was installed. Only counts are sent.
   */
  /** The counts last given, so a redraw does not blank the badges until the next push. */
  let counts: Record<string, number> = Object.fromEntries(
    shiver.rail.map((entry) => [entry.id, entry.unread])
  );

  const paintBadgesNow = () => paintBadges(counts);

  const paintBadges = (unread: Record<string, number>) => {
    counts = unread;

    for (const [id, badge] of badges) {
      const count = unread[id] ?? 0;

      badge.textContent = count > 99 ? '99+' : String(count);
      // the server on screen reports its own unread in its own client; a badge for it in Shiver's
      // rail would be a second, staler answer to a question already on the page
      badge.hidden = count === 0 || id === shiver.entryId;
    }
  };

  // first draw, once the badge painter it calls exists
  rebuild();

  paintBadges(Object.fromEntries(shiver.rail.map((entry) => [entry.id, entry.unread])));

  window.__SHIVER_UNREAD__ = paintBadges;

  return {
    open: () => setOpen(true),
    close: () => setOpen(false),
    isOpen: () => open
  };
}

type TileOptions = {
  label: string;
  content: Node;
  active?: boolean;
  className?: string;
  /** an unread badge, which overhangs the tile rather than sitting inside it */
  badge?: HTMLElement;
  onPick: () => void;
  /** a long press, which is this rail's version of desktop's right-click menu */
  onHold?: (at: number) => void;
  /** a server tile, which the rail may drag into a different place */
  reorderable?: boolean;
};

function tile({
  label,
  content,
  active,
  className,
  badge,
  onPick,
  onHold,
  reorderable
}: TileOptions) {
  const button = el('button', `tile${active ? ' active' : ''}${className ? ` ${className}` : ''}`);

  button.setAttribute('type', 'button');
  button.setAttribute('aria-label', label);
  button.title = label;
  button.append(content);

  if (badge) button.append(badge);

  button.addEventListener('click', (event) => {
    // A press already used for a menu or a drag must not also count as a tap. Read as a time, not
    // a flag: one left standing would swallow the next tap instead of this one's click, and that
    // click does not always arrive.
    const heldAt = Number(button.dataset.heldAt ?? 0);

    delete button.dataset.heldAt;

    if (heldAt && Date.now() - heldAt < CLICK_GRACE_MS) {
      event.preventDefault();
      event.stopPropagation();

      return;
    }

    onPick();
  });

  // A reorderable tile's hold is the rail's to interpret: held still it is a menu, held and moved
  // it is a drag, and only the rail can tell those apart. Everything else keeps the plain press.
  if (onHold && reorderable) {
    button.addEventListener('contextmenu', (event) => event.preventDefault());
    button.addEventListener('shiver-hold', (event) =>
      onHold((event as CustomEvent<number>).detail)
    );
  } else if (onHold) {
    installLongPress(button, onHold);
  }

  return button;
}

/**
 * Leaves for Shiver's own pages, carrying at most an entry id.
 *
 * A navigation rather than a call: a page on a server's origin has no Tauri IPC, by the same rule
 * that protects the desktop client, so this is the only way back — and it is why the rail can be
 * handed ids instead of addresses. Shiver resolves the id on its own side.
 */
function goHome(hash: string) {
  const base = shiverConfig?.home;

  if (!base) return;

  window.location.assign(base.replace(/#.*$/, '') + hash);
}

/* ──────────────────────────── the gestures ──────────────────────────── */

/**
 * Adds the level above Sharkord's own drawer.
 *
 * Sharkord binds its swipe handling as React props on the server view, which React delivers on the
 * bubble phase at its root container. Shiver listens on `window` in the capture phase, so it sees a
 * gesture first and can decide whether Sharkord should see it at all:
 *
 * - drawer closed → Sharkord's, untouched. Swipe right opens the channel list, as it always did.
 * - drawer open, swipe right → Shiver's. Its own handler would *close* the drawer, which is the one
 *   thing that must not happen here, so the gesture is swallowed and the rail opens instead.
 * - rail open, swipe either way → Shiver's, and swallowed: swipe left closes the rail rather than
 *   letting Sharkord open its member list behind it.
 *
 * Only `touchend` is ever swallowed. Sharkord tracks the gesture across start and move and acts on
 * end, so it keeps a consistent picture of a gesture it is not given.
 */
function installGestures(current: () => Rail | undefined) {
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastY = 0;

  const options = { capture: true, passive: true } as const;

  window.addEventListener(
    'touchstart',
    (event) => {
      // Shiver synthesizes a swipe to open Sharkord's drawer, and must not answer its own gesture
      if (!event.isTrusted) return;

      const touch = event.touches[0];

      if (!touch) return;

      startX = lastX = touch.clientX;
      startY = lastY = touch.clientY;
    },
    options
  );

  window.addEventListener(
    'touchmove',
    (event) => {
      if (!event.isTrusted) return;

      const touch = event.touches[0];

      if (!touch) return;

      lastX = touch.clientX;
      lastY = touch.clientY;
    },
    options
  );

  window.addEventListener(
    'touchend',
    (event) => {
      if (!event.isTrusted) return;

      const rail = current();

      if (!rail) return;

      const dx = lastX - startX;
      const dy = Math.abs(lastY - startY);

      // Sharkord's own test, so a gesture is never a swipe to one of us and a tap to the other
      if (Math.abs(dx) < SWIPE_THRESHOLD || Math.abs(dx) <= dy) return;

      if (rail.isOpen()) {
        if (dx < 0) rail.close();

        event.stopPropagation();

        return;
      }

      if (dx < 0) return;

      // the second swipe: Sharkord's drawer is showing, and its own handler would close it
      if (drawerIsOpen() && startedNearTheDrawer(startX)) {
        rail.open();
        event.stopPropagation();

        return;
      }

      // A page with no channel list at all — a sign-in page, or anything a server puts in front of
      // its client. Nothing there competes for a horizontal swipe, so *any* right swipe opens the
      // rail rather than only one starting at the edge.
      //
      // It used to require an edge start, which made the rail unreachable in practice: Android
      // claims the first ~24dp for its own back gesture, so the swipe that was supposed to open the
      // rail navigated back instead, on exactly the screen a user is most likely to want out of.
      if (!hasSidebar()) rail.open();
    },
    { capture: true }
  );
}

/** How long a press has to be held to mean "menu" rather than "open". */
const HOLD_MS = 500;
/** How long after a long press its own `click` may still arrive and need ignoring. */
const CLICK_GRACE_MS = 700;
/** How long Shiver waits for Sharkord's own channel menu before offering its own instead. */
const SHARKORD_MENU_GRACE_MS = 450;
/**
 * How long after a finger goes down on a channel row Shiver will still treat a menu appearing as
 * Sharkord's answer to that press.
 *
 * Wide on purpose. It only has to be shorter than the gap between two deliberate presses, and being
 * too narrow is what left two menus on screen.
 */
const SHARKORD_MENU_WINDOW_MS = 1500;
/** How often Shiver looks at whether the client has given up on its connection. */
const RECONNECT_POLL_MS = 2000;
/**
 * How long to wait before each attempt at getting the client to reconnect.
 *
 * Four tries, then Shiver stops and lets the server's own screen stand. Somewhere past that a retry
 * loop stops being helpful — the server may be down, or this network may not be going anywhere —
 * and quietly hammering it forever is worse than saying so.
 */
const RECONNECT_DELAYS_MS = [0, 3000, 8000, 15000];
/** How far a finger may drift during that hold and still count as a press rather than a scroll. */
const HOLD_SLOP = 10;

/**
 * A long press, standing in for desktop's right-click.
 *
 * Its own timer rather than the `contextmenu` event: Android fires that inconsistently on elements
 * that are not text or links, and it also brings the system's own selection menu with it. That
 * event is suppressed here for the same reason.
 */
function installLongPress(target: HTMLElement, onHold: (at: number) => void) {
  let timer = 0;
  let startX = 0;
  let startY = 0;

  const cancel = () => {
    window.clearTimeout(timer);
    timer = 0;
  };

  target.addEventListener('contextmenu', (event) => event.preventDefault());

  target.addEventListener(
    'touchstart',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      startX = touch.clientX;
      startY = touch.clientY;

      timer = window.setTimeout(() => {
        // read by the click handler, which must not also treat this press as a tap
        target.dataset.heldAt = String(Date.now());
        onHold(startY);
      }, HOLD_MS);
    },
    { passive: true }
  );

  target.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      if (
        Math.abs(touch.clientX - startX) > HOLD_SLOP ||
        Math.abs(touch.clientY - startY) > HOLD_SLOP
      ) {
        cancel();
      }
    },
    { passive: true }
  );

  target.addEventListener('touchend', cancel, { passive: true });
  target.addEventListener('touchcancel', cancel, { passive: true });
}

/**
 * Dragging a server tile into a different place in the rail.
 *
 * The desktop client reorders by dragging, and this is the same gesture where there is no pointer:
 * hold a tile until it lifts, then move it. Held still and released it is the menu instead, which
 * is the one thing that had to survive — a long press already meant something here, and taking that
 * away to add this would be a poor trade.
 *
 * The tiles are moved in the dom as the finger passes them, so the rail shows the order it will
 * keep rather than snapping to it on release. Nothing is sent anywhere: `__SHIVER_ORDER__` simply
 * reports where the tiles are, and the core notices on its next look. This page cannot call Shiver,
 * and a drag is not worth a navigation.
 */
function installRailDrag(
  nav: HTMLElement,
  shiver: ShiverConfig,
  openFolders: Set<string>,
  rebuild: () => void
) {
  let timer = 0;
  let held: HTMLElement | null = null;
  let startY = 0;
  let moved = false;
  /** the folder the held tile was in when it was picked up, so a change can be noticed on release */
  let startFolder: string | null = null;
  /** the tile the finger is resting on the middle of, which means "put these together" */
  let onto: HTMLElement | null = null;

  const clearOnto = () => {
    onto?.classList.remove('drop-onto');
    onto = null;
  };

  /** how far the held tile has been pushed from where the layout put it */
  let carriedBy = 0;

  /**
   * Keeps the held tile under the finger.
   *
   * Measured against where the layout would have put it rather than accumulated from the last
   * touch, and that is what makes it survive a reorder: passing another tile moves this one in the
   * dom, so a running total would leave it standing a whole tile away from the finger. Reading the
   * box back each time and taking off what has already been applied gives the distance afresh, so
   * the tile stays put under the finger however the rail rearranges itself beneath it.
   *
   * Scale is reapplied here because an inline transform replaces the one the `lifted` class carries,
   * and the two have to be written together. The centre is unaffected by it — that is what makes it
   * safe to measure.
   */
  const carry = (y: number) => {
    if (!held) return;

    const box = held.getBoundingClientRect();
    const home = box.top + box.height / 2 - carriedBy;

    carriedBy = y - home;
    held.style.transform = `translateY(${carriedBy}px) scale(1.12)`;
  };
  let holdAt = 0;

  /**
   * Everything a drag may pass over: folders, loose servers, and the servers inside open folders.
   *
   * A server can be dragged out of a folder and another dragged in, so members have to be part of
   * this. Where a tile *ends up* is what decides which it is — see `folderOf`.
   */
  const tiles = () => [...nav.querySelectorAll<HTMLElement>('.tile-folder, .tile-server')];

  /**
   * The rail's top level, which is the only part that has an order to report.
   *
   * A tile sitting in a folder's box is that folder's business; the servers inside keep their own
   * order and take no part in this one.
   */
  const topLevel = () =>
    [...nav.children].filter(
      (node): node is HTMLElement =>
        node instanceof HTMLElement &&
        (node.classList.contains('tile-folder') || node.classList.contains('tile-server'))
    );

  /** The folder a tile is currently sitting in, read from where it is rather than from what it was. */
  const folderOf = (tile: HTMLElement) =>
    tile.parentElement?.classList.contains('folder-contents')
      ? (tile.parentElement.dataset.folderId ?? null)
      : null;

  /**
   * Writes the order the tiles are now in back into the rail's own copy of it.
   *
   * A drag moves the dom and leaves the positions it was drawn from stale, which is fine until
   * something has to redraw — and then every tile jumps back to where it was before the drag. So
   * before any redraw, what the user did becomes what the rail believes. The core is told the same
   * thing separately, by `railState`; this is only so the next draw agrees with the screen.
   */
  const syncPositions = () => {
    topLevel().forEach((tile, index) => {
      const folderId = tile.dataset.folderId;

      if (folderId) {
        const folder = shiver.folders.find((candidate) => candidate.id === folderId);

        if (folder) folder.position = index;

        return;
      }

      const entry = shiver.rail.find((candidate) => candidate.id === tile.dataset.entryId);

      if (entry) entry.position = index;
    });

    // and inside each open folder, which keeps an order of its own
    for (const box of nav.querySelectorAll<HTMLElement>('.folder-contents')) {
      [...box.children].forEach((child, index) => {
        if (!(child instanceof HTMLElement)) return;

        const entry = shiver.rail.find((candidate) => candidate.id === child.dataset.entryId);

        if (entry) entry.position = index;
      });
    }
  };

  /**
   * Keeps an open folder's servers directly under it.
   *
   * They live in a box of their own rather than loose in the rail, and a drag moves tiles — so
   * without this a folder dragged past its neighbour would leave its contents behind, sitting under
   * whatever tile happened to take its place.
   */
  const keepContentsWithFolders = () => {
    for (const tile of nav.querySelectorAll<HTMLElement>('.tile-folder')) {
      const id = tile.dataset.folderId;

      if (!id) continue;

      const contents = nav.querySelector<HTMLElement>(`.folder-contents[data-folder-id="${id}"]`);

      if (contents && tile.nextSibling !== contents) nav.insertBefore(contents, tile.nextSibling);
    }
  };

  const railState = () => ({
    order: topLevel()
      .map((tile) =>
        tile.dataset.folderId
          ? ({ kind: 'folder', id: tile.dataset.folderId } as const)
          : ({ kind: 'server', id: tile.dataset.entryId ?? '' } as const)
      )
      .filter((item) => !!item.id),
    // handed over once: the core stores them, and asking twice would fight the user's next move
    moves: pendingMoves.splice(0, pendingMoves.length),
    creates: pendingCreates.splice(0, pendingCreates.length)
  });

  window.__SHIVER_RAIL_STATE__ = railState;

  const release = () => {
    held?.classList.remove('lifted');

    if (held) {
      held.style.transform = '';
      held.style.transition = '';
    }

    carriedBy = 0;
    window.clearTimeout(timer);

    timer = 0;
    held = null;
    moved = false;
    startFolder = null;

    clearOnto();
  };

  nav.addEventListener(
    'touchstart',
    (event) => {
      const touch = event.touches[0];
      const tile = (event.target as Element | null)?.closest?.('.tile-server');

      release();

      if (!touch || !(tile instanceof HTMLElement)) return;

      startY = touch.clientY;
      holdAt = startY;

      timer = window.setTimeout(() => {
        held = tile;
        moved = false;
        startFolder = folderOf(tile);

        tile.classList.add('lifted');

        // A tile that jumps to the finger is the rail saying it has been picked up. It travels
        // there over the class's own transition, so the lift is a movement rather than a flicker.
        carriedBy = 0;

        carry(startY);
      }, HOLD_MS);
    },
    { passive: true }
  );

  nav.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      // not lifted yet: a finger that wanders is scrolling the rail, not pressing a tile
      if (!held) {
        if (Math.abs(touch.clientY - startY) > HOLD_SLOP) release();

        return;
      }

      if (Math.abs(touch.clientY - startY) > HOLD_SLOP) moved = true;

      // From here the tile is being carried, not settling into place: an easing meant for the lift
      // would have it trailing the finger.
      held.style.transition = 'none';

      carry(touch.clientY);

      // Dragged clear of the group it was in, which takes it out at once and without having to find
      // a tile to land on.
      //
      // An open folder's own tile is a slim bar, so "just above the folder" is a few pixels of
      // travel — not something to ask a finger to hit, and the reason taking a server out felt
      // impossible. Leaving the box the members are drawn in is the plain reading of the gesture,
      // and it is what someone pulling a server out of a group is already doing.
      const group = held.parentElement?.classList.contains('folder-contents')
        ? held.parentElement
        : null;

      if (group) {
        const inside = group.getBoundingClientRect();

        if (touch.clientY < inside.top || touch.clientY > inside.bottom) {
          const above = touch.clientY < inside.top;
          const folderTile = [...nav.children].find(
            (node): node is HTMLElement =>
              node instanceof HTMLElement && node.dataset.folderId === group.dataset.folderId
          );

          clearOnto();
          nav.insertBefore(held, above ? (folderTile ?? group) : group.nextSibling);
          held.classList.remove('in-folder');

          return;
        }
      }

      // A folder cannot go inside a folder, so it only ever passes over the top level.
      const holdingFolder = held.classList.contains('tile-folder');

      const over = tiles().find((tile) => {
        if (tile === held) return false;
        if (holdingFolder && folderOf(tile)) return false;

        const box = tile.getBoundingClientRect();

        return touch.clientY >= box.top && touch.clientY <= box.bottom;
      });

      if (!over) return;

      const box = over.getBoundingClientRect();
      const offset = touch.clientY - box.top;

      // The middle of a tile is not a place in the order, it is the tile itself: resting there
      // means "these two belong together" — a folder made of the pair, or the server put into the
      // folder it is over. The edges still mean before and after, so ordinary reordering is
      // unchanged and there is no new gesture to learn.
      //
      // Wider to leave than to enter. A tile is about forty pixels tall, so a band measured the
      // same way in both directions is a few pixels of finger, and drifting a little while deciding
      // would drop the tile back into the order. Once the rail has said these two would be grouped,
      // it takes leaving the tile to take it back.
      const grouping =
        onto === over
          ? offset > box.height * 0.1 && offset < box.height * 0.9
          : offset > box.height * 0.25 && offset < box.height * 0.75;

      if (!holdingFolder && grouping) {
        if (onto !== over) {
          clearOnto();

          onto = over;
          over.classList.add('drop-onto');
        }

        return;
      }

      clearOnto();

      // above the midpoint it goes before that tile, below it after — so passing a tile swaps with
      // it once rather than jittering across the boundary
      const before = touch.clientY < box.top + box.height / 2;

      // Into whichever list the tile it passed belongs to. This is what takes a server out of a
      // folder and puts one in: the drop decides the scope, and the class follows the dom rather
      // than the other way round.
      const into = over.parentElement ?? nav;

      into.insertBefore(held, before ? over : over.nextSibling);
      held.classList.toggle('in-folder', !!folderOf(held));
      keepContentsWithFolders();
    },
    { passive: true }
  );

  nav.addEventListener(
    'touchend',
    () => {
      const tile = held;
      const wasDrag = moved;
      const was = startFolder;
      const target = onto;

      release();

      if (!tile) return;

      // Either way the press has been used, and the click behind it must not also open the server.
      tile.dataset.heldAt = String(Date.now());

      const serverId = tile.dataset.entryId;

      // Dropped on the middle of another tile: a folder of the two, or into the folder it landed on.
      if (wasDrag && serverId && target && target !== tile) {
        const intoFolder = target.dataset.folderId;

        if (intoFolder) {
          pendingMoves.push({ serverId, folderId: intoFolder });
          entryFolder(shiver, serverId, intoFolder);
          openFolders.add(intoFolder);
        } else if (target.dataset.entryId) {
          // The rail names the folder so it can draw it at once rather than waiting for the core to
          // answer; Shiver keeps the name it is given.
          const id = newFolderId();

          pendingCreates.push({
            id,
            name: 'New folder',
            memberIds: [target.dataset.entryId, serverId]
          });

          syncPositions();

          // where the tile it was dropped on was standing, so the folder appears under the finger
          const place = topLevel().indexOf(target);

          shiver.folders.push({ id, name: 'New folder', position: place, expanded: true });
          entryFolder(shiver, target.dataset.entryId, id);
          entryFolder(shiver, serverId, id);
          openFolders.add(id);
        }

        syncPositions();
        rebuild();

        return;
      }

      // Dragged out of a folder, or into one: the order alone cannot say so, because a server
      // inside a folder is not part of the order. This is the change that has to be spelled out.
      const landedIn = folderOf(tile);

      if (wasDrag && serverId && landedIn !== was) {
        pendingMoves.push({ serverId, folderId: landedIn });
        entryFolder(shiver, serverId, landedIn);

        // Taking a server out can leave a folder holding one, which is not a folder any more. The
        // redraw is what dissolves it, and it is safe here only because the positions have just
        // been made to agree with the tiles.
        syncPositions();
        rebuild();
      }

      // held still: what a long press meant before this existed
      if (!wasDrag) tile.dispatchEvent(new CustomEvent('shiver-hold', { detail: holdAt }));
    },
    { passive: true }
  );

  nav.addEventListener('touchcancel', release, { passive: true });
}

/**
 * A folder's face: the first few of its servers, tiled.
 *
 * The same shorthand the desktop rail uses, and the reason a folder is recognisable without being
 * opened. Four at most, because below that size an icon stops being a picture of anything.
 */
function folderIcon(members: RailEntry[], expanded: boolean) {
  // Open, the tile steps out of the way: a slim bar instead of a picture, because the servers it
  // holds are drawn underneath it and showing them twice says nothing. The desktop rail does the
  // same, and this is the shape the two are meant to share.
  if (expanded) return el('span', 'folder-open');

  const grid = el('span', 'folder-grid');

  for (const member of members.slice(0, 4)) {
    const cell = el('span', 'folder-cell');

    if (member.icon) {
      const img = image(member.icon);

      cell.append(img);
    } else {
      cell.textContent = initials(member.name).slice(0, 1);
    }

    grid.append(cell);
  }

  return grid;
}

/**
 * A folder's menu, which offers what this page can actually carry out.
 *
 * Renaming and deleting are Shiver's own records rather than anything on this server, and a page on a
 * server's origin cannot call Shiver — so, as with removing a server, the menu leaves for Shiver's own
 * screen and the change is made there. Folders are made and unmade rarely enough that the trip is a
 * fair price for not letting a server's page rewrite the rail's structure.
 */
function openFolderMenu(root: ShadowRoot, folder: RailFolder, at: number) {
  root.querySelector('.menu')?.remove();

  const menu = el('div', 'menu');

  menu.style.top = `${Math.max(8, at - 24)}px`;

  const item = el('button', 'menu-item');

  item.setAttribute('type', 'button');
  item.textContent = `Manage "${folder.name}"`;
  item.addEventListener('click', () => {
    menu.remove();
    goHome('#settings');
  });

  menu.append(item);
  root.append(menu);

  const dismiss = (event: Event) => {
    if (event.target instanceof Node && menu.contains(event.target)) return;

    menu.remove();
    root.removeEventListener('click', dismiss, true);
  };

  root.addEventListener('click', dismiss, true);
}

/**
 * The rail's context menu, matching the desktop client's right-click menu item for item.
 *
 * Two of its actions can only be carried out by the server's own client — marking every channel
 * read means selecting each one, and signing out means clearing what that page persisted — so those
 * appear only for the server on screen. Desktop shows them for any server because every server has
 * a webview there; here there is one. The two that desktop has and this cannot are "Sign in", which
 * needs credentials Shiver does not hold on Android, and "Move out of folder", there being no folders.
 *
 * Everything else is a navigation to Shiver's own page carrying an action and an id, for the same
 * reason the rail itself navigates: this page has no way to call Shiver.
 */
function openServerMenu(
  root: ShadowRoot,
  entry: RailEntry,
  active: boolean,
  at: number,
  closeRail: () => void,
  folders: RailFolder[],
  moved: (folderId: string | null) => void
) {
  root.querySelector('.menu')?.remove();

  const menu = el('div', 'menu');

  menu.style.top = `${Math.max(8, at - 24)}px`;

  const add = (label: string, run: () => void) => {
    const item = el('button', 'menu-item');

    item.setAttribute('type', 'button');
    item.textContent = label;
    item.addEventListener('click', () => {
      menu.remove();
      run();
    });

    menu.append(item);
  };

  if (!active) add('Open', () => goHome(`#open=${encodeURIComponent(entry.id)}`));

  if (active) {
    add('Mark all as read', () => {
      markAllChannelsRead();
      closeRail();
    });
  }

  add('Refresh name and icon', () => goHome(`#do=refresh:${encodeURIComponent(entry.id)}`));

  menu.append(el('div', 'menu-divider'));

  // Which folder a server sits in is the one part of the rail's shape this page may change, and it
  // is done from here rather than by dragging: the tiles are 48px apart, and asking a finger to
  // land inside one to mean "into this folder" rather than "next to it" is a poor gesture. Making,
  // renaming and deleting folders stays on Shiver's own screen, which a server's page cannot reach.
  for (const folder of folders) {
    if (folder.id === entry.folderId) continue;

    add(`Move to "${folder.name}"`, () => moved(folder.id));
  }

  if (entry.folderId) add('Take out of folder', () => moved(null));

  if (folders.length || entry.folderId) menu.append(el('div', 'menu-divider'));

  if (active) add('Log out', signOut);

  add('Remove from Shiver', () => goHome(`#do=remove:${encodeURIComponent(entry.id)}`));

  root.append(menu);

  // one tap anywhere else closes it, the way a menu should behave
  const dismiss = (event: Event) => {
    if (event.target instanceof Node && menu.contains(event.target)) return;

    menu.remove();
    root.removeEventListener('click', dismiss, true);
  };

  root.addEventListener('click', dismiss, true);
}

/* ────────────────────── muting a channel ───────────────────── */

/**
 * A long press on a channel opens Shiver's mute menu.
 *
 * Desktop adds its mute item to Sharkord's own right-click menu. There is no touch equivalent —
 * `contextmenu` does not fire from a finger — so on Android Shiver brings its own menu, held to the
 * same rule as the rail's: it does only what desktop's item does.
 *
 * Bound once at the document, on the way down, because Sharkord rebuilds these rows constantly and
 * a listener attached to a row would go with it.
 */
function installChannelMenu() {
  let timer = 0;
  let startX = 0;
  let startY = 0;
  let row: HTMLElement | null = null;
  /**
   * When the last long press opened the menu.
   *
   * A timestamp rather than a flag, and that is the whole point. The press's own `click` has to be
   * swallowed or it selects the channel underneath, but a flag left standing swallows whatever the
   * user taps next instead — which is what happened: choosing "Mute" returns early without clearing
   * it, and the following tap anywhere in the app went missing.
   */
  let heldAt = 0;
  /** the channel a press is waiting on, while Shiver finds out whether Sharkord will offer a menu */
  let pending: SharkordChannel | null = null;
  let fallback = 0;
  /**
   * The row a finger went down on, and when — kept until the menu question is settled.
   *
   * Separate from `row`, which the hold timer clears, because Sharkord's menu can arrive *before*
   * Shiver has even decided a press happened. Measured on a phone: Sharkord's own menu lands 521ms
   * after touchstart and Shiver's hold fires at ~551ms, so the code that waited for Shiver's press to
   * come first had nothing to match the menu against, ignored it, and opened a second menu on top
   * — which is exactly the stacked pair this was supposed to have removed.
   */
  let pressedRow: HTMLElement | null = null;
  let pressedAt = 0;

  const cancel = () => {
    window.clearTimeout(timer);
    timer = 0;
    row = null;
  };

  const settle = () => {
    window.clearTimeout(fallback);
    fallback = 0;
    pending = null;
    pressedRow = null;
    pressedAt = 0;
  };

  /**
   * Puts Shiver's mute item into Sharkord's own channel menu.
   *
   * Sharkord shows that menu to anyone who can manage channels, on the same long press Shiver uses,
   * so both used to open and the user got two menus stacked on each other. There is only ever one
   * now: where Sharkord has a menu Shiver joins it, exactly as the desktop client does with the
   * right-click menu, and Shiver's own menu below is the fallback for everyone else — a member with
   * no permissions gets no menu from Sharkord at all (`context-menus/channel` returns its children
   * untouched), and muting is not a permission.
   */
  const joinSharkordMenu = (menu: HTMLElement, channel: SharkordChannel) => {
    if (menu.querySelector(`.${SHIVER_MENU_ITEM}`)) return;

    const sibling = menu.querySelector<HTMLElement>('[role="menuitem"]');
    const item = document.createElement('div');
    const isMuted = (window.__SHIVER_MUTED__?.() ?? []).includes(channel.id);

    item.setAttribute('role', 'menuitem');
    item.tabIndex = -1;
    // borrowed from a sibling so the item looks like the menu it is joining rather than like Shiver
    item.className = `${sibling?.className ?? ''} ${SHIVER_MENU_ITEM}`.trim();
    item.textContent = isMuted ? 'Unmute in Shiver' : 'Mute in Shiver';

    item.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();

      window.__SHIVER_TOGGLE_MUTE__?.(channel.id);

      // let the page close its own menu the way it would for any other item
      document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });

    menu.appendChild(item);
  };

  // Sharkord's menu is mounted around the same time as Shiver's press, so Shiver watches for it rather
  // than guessing. Either order is handled: whenever a menu turns up while a finger is or has just
  // been on a channel row, that menu is the one the user gets, Shiver's item joins it, and Shiver's own
  // menu is called off — cancelled if it has not opened yet, closed if it has.
  new MutationObserver((records) => {
    if (!pressedRow || Date.now() - pressedAt > SHARKORD_MENU_WINDOW_MS) return;

    for (const record of records) {
      for (const node of record.addedNodes) {
        if (!(node instanceof HTMLElement)) continue;

        const menu = node.matches('[role="menu"]')
          ? node
          : node.querySelector<HTMLElement>('[role="menu"]');

        if (!menu) continue;

        const channel = pending ?? channelOfRow(pressedRow);

        if (!channel) return;

        joinSharkordMenu(menu, channel);

        // the press has been used, so the click behind it must not also select the channel
        heldAt = Date.now();

        cancel();
        closeChannelMenu();
        settle();

        return;
      }
    }
  }).observe(document.body, { childList: true, subtree: true });

  document.addEventListener(
    'touchstart',
    (event) => {
      const target = (event.target as Element | null)?.closest?.(
        `${CHANNEL_ITEM}, ${MESSAGE_ITEM}`
      );
      const touch = event.touches[0];

      cancel();

      // A touch *inside* the revealed toolbar is the user pressing one of its buttons, and a touch
      // in whatever that button opened — the emoji picker, the "more" menu, a delete confirmation,
      // all portalled to the body — is the user finishing that. Clearing on those took the toolbar
      // away under the finger before the tap could land, so every action dismissed instead of
      // acting. Only a touch somewhere else means the user is done with it.
      const within = event.target instanceof Element ? event.target : null;
      const keeping =
        within?.closest(
          `.${MESSAGE_ACTIONS_CLASS}, [role="menu"], [role="dialog"], [role="listbox"], [data-radix-popper-content-wrapper]`
        ) ?? null;

      if (!keeping) clearMessageActions();

      if (!touch || !(target instanceof HTMLElement)) return;

      row = target;
      startX = touch.clientX;
      startY = touch.clientY;

      // remembered from the moment of contact, because Sharkord's menu does not wait for Shiver's
      // hold to fire before it appears
      pressedRow = target;
      pressedAt = Date.now();

      timer = window.setTimeout(() => {
        const held = row;

        cancel();

        if (!held) return;

        heldAt = Date.now();

        // A message: Sharkord already has every action for one, in a toolbar it only reveals on
        // hover — which a finger never produces, so on a phone there was no way to reply to,
        // edit or react to anything. Shiver shows that toolbar instead of building its own.
        if (held.matches(MESSAGE_ITEM)) {
          held.classList.add(MESSAGE_ACTIONS_CLASS);

          return;
        }

        const channel = channelOfRow(held);

        if (!channel) return;

        pending = channel;

        // Sharkord's own long press is slower than Shiver's, so its menu lands after this one would.
        // Waiting lets it win where it has something to show, and Shiver only steps in when nothing
        // arrives.
        fallback = window.setTimeout(() => {
          const channel = pending;

          settle();

          if (!channel) return;

          // Sharkord's menu may be open already, from a press a moment ago on the same row: Radix
          // moves the menu it has rather than making another, so nothing is added to the document
          // and the observer above never hears about it. Desktop drew a second menu beside a
          // perfectly good one for exactly this reason; mobile is the same code shape.
          const open = openMenuOnScreen();

          if (open) {
            joinSharkordMenu(open, channel);

            return;
          }

          openChannelMenu(channel, startX, startY);
        }, SHARKORD_MENU_GRACE_MS);
      }, HOLD_MS);
    },
    { passive: true, capture: true }
  );

  document.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      if (
        Math.abs(touch.clientX - startX) > HOLD_SLOP ||
        Math.abs(touch.clientY - startY) > HOLD_SLOP
      ) {
        cancel();
      }
    },
    { passive: true, capture: true }
  );

  document.addEventListener('touchend', cancel, { passive: true, capture: true });
  document.addEventListener('touchcancel', cancel, { passive: true, capture: true });

  document.addEventListener(
    'click',
    (event) => {
      if (!heldAt) return;

      const target = event.target instanceof Node ? event.target : null;

      // the menu is in a closed shadow root, so a tap inside it arrives retargeted to the host —
      // that one is the menu's own and has to be let through, as is anything in Sharkord's menu
      if (target && menuHost?.contains(target)) return;
      if (target instanceof Element && target.closest('[role="menu"]')) return;

      const wasThePress = Date.now() - heldAt < CLICK_GRACE_MS;

      heldAt = 0;

      // only the press's own click is worth swallowing; anything later is a tap the user meant
      if (!wasThePress) return;

      event.preventDefault();
      event.stopPropagation();
    },
    true
  );
}

/** Hides any message toolbar Shiver revealed, so only the message last held shows one. */
function clearMessageActions() {
  for (const shown of document.querySelectorAll(`.${MESSAGE_ACTIONS_CLASS}`)) {
    shown.classList.remove(MESSAGE_ACTIONS_CLASS);
  }
}

/**
 * Servers the user has moved between folders, waiting for the core's next look.
 *
 * Module level rather than inside the rail, because the rail is rebuilt whenever a folder opens or
 * shuts and a move must not be lost to a redraw.
 */
const pendingMoves: { serverId: string; folderId: string | null }[] = [];

/**
 * Sharkord's own menu, if one is on screen.
 *
 * `data-state` is Radix's own word for it, and a menu it has closed can stay in the document — so
 * this measures as well. Shiver's own menu is in a closed shadow root and cannot be seen from here,
 * which is what makes the question safe to ask.
 */
function openMenuOnScreen(): HTMLElement | null {
  for (const menu of document.querySelectorAll<HTMLElement>('[role="menu"]')) {
    if (menu.dataset.state === 'closed') continue;

    const box = menu.getBoundingClientRect();

    if (box.width > 0 && box.height > 0) return menu;
  }

  return null;
}

/** Folders made by dropping one server onto another, waiting for the core's next look. */
const pendingCreates: { id: string; name: string; memberIds: string[] }[] = [];

/** The host of the channel menu, kept so a tap inside it can be told from a tap on the page. */
let menuHost: HTMLElement | null = null;

function openChannelMenu(channel: SharkordChannel, x: number, y: number) {
  closeChannelMenu();

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const style = document.createElement('style');
  const menu = document.createElement('div');
  const item = document.createElement('button');
  const sheet = document.createElement('div');
  const isMuted = (window.__SHIVER_MUTED__?.() ?? []).includes(channel.id);

  host.id = MENU_HOST_ID;

  style.textContent = `
:host { position: fixed; inset: 0; z-index: 2147483646; }
.sheet { position: absolute; inset: 0; }
.menu { position: absolute; min-width: 200px; max-width: 70vw; padding: 6px; border-radius: 12px;
  border: 1px solid rgb(255 255 255 / 12%); background: #1f1f1f; color: #fafafa;
  box-shadow: 0 12px 32px rgb(0 0 0 / 55%);
  font: 500 14px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
.menu-item { display: block; width: 100%; padding: 11px 12px; border: none;
  border-radius: 8px; background: none; color: inherit; font: inherit; text-align: left; }
.menu-item:active { background: var(--shiver-surface-hover, #333333); }
`;

  item.className = 'menu-item';
  item.type = 'button';
  item.textContent = isMuted ? `Unmute #${channel.name}` : `Mute #${channel.name}`;

  item.addEventListener('click', () => {
    window.__SHIVER_TOGGLE_MUTE__?.(channel.id);
    closeChannelMenu();
  });

  menu.className = 'menu';
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - 220))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - 80))}px`;
  menu.append(item);

  sheet.className = 'sheet';

  const openedAt = Date.now();

  // One tap anywhere else closes it, the way a menu should behave — but not the tap that opened it.
  // This overlay covers the viewport, so the finger that was held on a channel comes up over the
  // sheet, and without the grace the menu would dismiss itself on the press that asked for it.
  sheet.addEventListener('click', () => {
    if (Date.now() - openedAt < CLICK_GRACE_MS) return;

    closeChannelMenu();
  });

  root.append(style, sheet, menu);
  document.body.append(host);

  menuHost = host;
}

function closeChannelMenu() {
  menuHost?.remove();
  menuHost = null;
}

/**
 * The channel a row stands for.
 *
 * Sharkord's rows carry no channel id — only `data-testid="channel-item"` and the name as text — so
 * the name is the only way across, and direct messages are left out because muting one is not
 * something Shiver offers. Two channels sharing a name is the known cost of that, and it is why the
 * mute itself is stored against the id rather than the name.
 */
function channelOfRow(row: HTMLElement): SharkordChannel | null {
  const name = rowName(row);

  if (!name) return null;

  const channels = sharkordStore()?.getState().channels ?? [];

  return channels.find((channel) => !channel.isDm && channel.name === name) ?? null;
}

/**
 * A channel row's name, without the unread count sitting next to it.
 *
 * `textContent` does not care that the count is hidden by css, so a channel with anything unread
 * reads as "general3" and matches no channel at all. Left alone that breaks muting for exactly the
 * channels a user is most likely to want muted, and it un-dims a muted one the moment it has
 * something to say.
 */
function rowName(row: HTMLElement) {
  const text = row.textContent?.trim() ?? '';
  const count = row.querySelector(UNREAD_COUNT)?.textContent?.trim() ?? '';

  return (count && text.endsWith(count) ? text.slice(0, -count.length) : text).trim();
}

/* ─────────────────── the companion plugin ─────────────────── */

type PluginResult = { mutedChannels?: number[]; status?: string } | null;

type PluginResponse = {
  source?: string;
  id?: string;
  ok?: boolean;
  result?: PluginResult;
};

/** Set once this device's own mutes have been folded into the server's list, and never again. */
const MUTES_ADOPTED = 'shiver-mutes-adopted';

const PLUGIN_REQUEST = 'shiver-bridge';
const PLUGIN_RESPONSE = 'shiver-plugin';
const PLUGIN_TIMEOUT_MS = 8000;
/** the plugin bundle is imported by the client after connecting, so its relay appears late */
const PLUGIN_WAIT_MS = 15000;

/**
 * Calls one of the Shiver plugin's actions through its client bundle.
 *
 * The bridge cannot call `executePluginAction` directly: Sharkord reads the plugin id off the
 * calling stack frame, and this script is not served from /plugin-bundle. So the request is posted
 * into the page and the plugin's own bundle makes the call. Identical to the desktop bridge, which
 * is deliberate — both talk to the same plugin, and the shape of that conversation is the plugin's.
 */
function callPlugin(action: string, payload?: unknown): Promise<PluginResult> {
  if (!window.__SHIVER_PLUGIN__) return Promise.resolve(null);

  return new Promise((resolve) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;

    const done = (value: PluginResult) => {
      window.removeEventListener('message', onMessage);
      window.clearTimeout(timer);
      resolve(value);
    };

    const onMessage = (event: MessageEvent) => {
      if (event.source !== window || event.origin !== window.location.origin) return;

      const data = event.data as PluginResponse | undefined;

      if (!data || data.source !== PLUGIN_RESPONSE || data.id !== id) return;

      done(data.ok ? data.result ?? null : null);
    };

    const timer = window.setTimeout(() => done(null), PLUGIN_TIMEOUT_MS);

    window.addEventListener('message', onMessage);
    window.postMessage({ source: PLUGIN_REQUEST, id, action, payload }, window.location.origin);
  });
}

function waitForPlugin(): Promise<boolean> {
  if (window.__SHIVER_PLUGIN__) return Promise.resolve(true);

  return new Promise((resolve) => {
    const started = Date.now();

    const timer = window.setInterval(() => {
      if (window.__SHIVER_PLUGIN__) {
        window.clearInterval(timer);
        resolve(true);

        return;
      }

      if (Date.now() - started > PLUGIN_WAIT_MS) {
        window.clearInterval(timer);
        resolve(false);
      }
    }, 500);
  });
}

/**
 * Reconciles this device's mutes with the server's copy, once, on connect.
 *
 * The two are unioned rather than one overwriting the other: a mute made on this phone while the
 * server had no plugin, and a mute made on a desktop, are both things the user asked for. After
 * this the server's copy is the truth.
 *
 * Returns null when the plugin is not installed, which is the normal case for a stock server, and
 * then Shiver keeps using its local list exactly as before.
 */
/**
 * Whether this device has already folded its own mutes into the server's list.
 *
 * Kept in the page's own storage, which is per server and per device — exactly the scope of the
 * question. Storage being unavailable answers "yes": adopting again is the failure that matters,
 * and forgetting a mute made before the plugin existed is the smaller loss.
 */
const hasAdoptedMutes = () => {
  try {
    return localStorage.getItem(MUTES_ADOPTED) === '1';
  } catch {
    return true;
  }
};

const rememberAdoptedMutes = () => {
  try {
    localStorage.setItem(MUTES_ADOPTED, '1');
  } catch {
    // nothing to do; the worst of it is one more union on the next connection
  }
};

async function syncMutesWithPlugin(local: number[]): Promise<number[] | null> {
  if (!(await waitForPlugin())) return null;

  const remote = await callPlugin('getMutedChannels');

  if (!remote) return null;

  // Filtered rather than taken as read. The list is a row the user's own client writes through the
  // plugin, so it is a server's answer either way — and a channel id that is not a positive integer
  // would sit in Shiver's mute set forever, matching nothing and never removable from the menu.
  const theirs = (remote.mutedChannels ?? []).filter(
    (id): id is number => Number.isInteger(id) && id > 0
  );
  // The server's list is the answer, not a suggestion. It used to be *unioned* with this device's
  // own on every connection, which meant an unmute could never travel: unmute a channel on the
  // desktop, open the phone, and the phone still had it muted locally, so the union put it back —
  // and then wrote that back to the server, re-muting it everywhere. A mute could spread and never
  // be taken away.
  if (hasAdoptedMutes()) return [...theirs].sort((a, b) => a - b);

  // Except once. The first time this device meets the plugin its own mutes have never been sent
  // anywhere — they were made before the plugin existed, or on a server that did not have it — and
  // dropping them silently is not Shiver's call to make. So they are folded in exactly once, written
  // back, and from then on the server is simply believed.
  const merged = [...new Set([...theirs, ...local])].sort((a, b) => a - b);
  const sameAsRemote =
    merged.length === theirs.length && merged.every((id) => theirs.includes(id));

  if (!sameAsRemote) {
    await callPlugin('setMutedChannels', { mutedChannels: merged });
  }

  rememberAdoptedMutes();

  return merged;
}

/**
 * Gives this server the endpoint it should wake the phone on.
 *
 * Only ever this server's own, and only when there is a plugin to take it — a stock server has no
 * way to be told, which is why push needs the companion plugin at all. The plugin checks the url
 * server-side before storing it, because a url this server will later fetch is one it has to look
 * at first.
 */
async function registerPushWithPlugin(endpoint: string): Promise<void> {
  if (!(await waitForPlugin())) return;

  await callPlugin('setPushEndpoint', { endpoint });
}

function pushMutesToPlugin(mutedChannels: number[]) {
  callPlugin('setMutedChannels', { mutedChannels }).catch(() => undefined);
}

/**
 * Lets a held message show the actions Sharkord already has for it.
 *
 * Sharkord reveals reply, edit, react, pin and delete in a toolbar keyed on `group-hover`, and a
 * finger never hovers, so on a phone none of them could be reached at all. Rather than rebuild any
 * of that, Shiver matches the toolbar by the tailwind class still sitting in its class list and shows
 * that same element — every action stays Sharkord's, including which of them the user is allowed.
 */
function styleMessageActions() {
  ensureStyle(ACTIONS_STYLE_ID).textContent = `
.${MESSAGE_ACTIONS_CLASS} [class*="group-hover:flex"] { display: flex !important; }
`;
}

/* ────────────────────── coming back from the background ───────────────────── */

/**
 * Puts the client back on its own feet after Android has had the app asleep.
 *
 * Sharkord reconnects by itself — five tries over about twenty seconds — but those are timers, and
 * a backgrounded webview does not get to run timers. So the whole budget burns down while the phone
 * is asleep and the user comes back not to a client reconnecting but to one that already gave up,
 * asking them to press a button. That is the screen this removes.
 *
 * Shiver does not reconnect anything itself. It presses the same button the user would, which returns
 * the client to its connect screen, where the auto-login Shiver seeded takes it the rest of the way —
 * the server's own mechanism, driven at the moment the user actually wants it.
 *
 * The button is the permission. A ban renders that screen with no button at all (`canReconnect` is
 * false in `screens/disconnected`), so there is nothing for this to press and Shiver does not argue
 * with the server about it.
 */
function installAutoReconnect() {
  // Kept in sessionStorage, not in a variable, because one of the things Shiver does about a failed
  // connection is reload the page — and a reload takes every variable in this file with it. Held in
  // memory the budget below reset itself on each reload, so a server whose stored session no longer
  // works would reload, fail, reload, forever, flashing its login screen on the way past. This is
  // the counter that survives the thing it is counting.
  let attempts = readAttempts();
  let nextAttemptAt = 0;
  let showing = false;
  /** a reachability probe is in flight; ticks step aside rather than stacking up behind it */
  let working = false;
  /** when the connect form was first seen, so a page still loading is not read as a failure */
  let formSince = 0;

  /** The client is up when its own sidebar is in the document; nothing else is a reliable tell. */
  const connected = () => !!document.querySelector(SIDEBAR);

  /**
   * The "try again" button on the screen Sharkord shows once it has stopped trying.
   *
   * Matched on the icon's own class rather than on the label, which is translated, and scoped to a
   * page with no client on it so a refresh button inside the running app cannot be hit by accident.
   */
  const retryButton = () => {
    const icon = document.querySelector('button svg[class*="lucide-refresh"]');

    return icon instanceof SVGElement ? icon.closest('button') : null;
  };

  /** Sharkord's own connect form, which is where a failed attempt leaves the client. */
  const signInForm = () => !!document.querySelector('input[type="password"]');

  const show = () => {
    if (showing) return;

    showing = true;
    showReconnecting();
  };

  const hide = () => {
    if (!showing) return;

    showing = false;
    hideReconnecting();
  };

  const tick = () => {
    if (document.hidden || working) return;

    if (connected()) {
      attempts = 0;
      nextAttemptAt = 0;
      formSince = 0;

      forgetAttempts();
      hide();

      return;
    }

    // Sharkord is still working through its own retry countdown and says so. That is the client
    // behaving correctly, and covering it would only replace one honest message with another.
    if (document.querySelector('[role="alertdialog"]')) {
      hide();

      return;
    }

    const button = retryButton();
    const form = signInForm();

    // mid-load, or a screen Shiver has no business touching
    if (!button && !form) {
      formSince = 0;

      return;
    }

    // A fresh page shows the connect form for a moment before Sharkord's auto-login takes it away,
    // and that moment is not a failure. Only a form still standing on the next look is one.
    if (form && !button) {
      if (!formSince) {
        formSince = Date.now();

        return;
      }

      if (Date.now() - formSince < RECONNECT_POLL_MS) return;
    }

    // Only a session Shiver is holding makes any of this possible. Without one — the user signed in
    // on the server's own page, or signed out — the connect form is the honest place to be.
    const session = shiverConfig?.session;

    if (!session || attempts >= RECONNECT_DELAYS_MS.length) {
      hide();

      return;
    }

    show();

    if (!nextAttemptAt) nextAttemptAt = Date.now() + RECONNECT_DELAYS_MS[attempts];

    if (Date.now() < nextAttemptAt) return;

    attempts += 1;
    nextAttemptAt = 0;

    rememberAttempts(attempts);

    // Put the sign-in back before pressing anything. Sharkord clears its own auto-login and token
    // whenever a connect fails, so by the time either of these screens is up Shiver's seeding is
    // already gone, and the button alone would land the user on the connect form asking for a
    // password. Seeded first, the controller finds a token the moment the button clears
    // `disconnectInfo` and re-runs its effect — Shiver supplies what it holds, Sharkord connects.
    if (button) {
      seedSession(session);
      button.click();

      return;
    }

    // Already on the connect form, which means the last attempt failed. Nothing on that screen
    // re-runs the auto-login — its effect only reacts to a disconnect clearing — so the way back is
    // a fresh load. That is only safe once the server is answering again: reloading into a dead
    // network replaces a form the user could still use with a browser error page.
    working = true;

    reachable()
      .then((ok) => {
        working = false;

        if (!ok) return;

        seedSession(session);
        location.reload();
      })
      .catch(() => {
        working = false;
      });
  };

  window.setInterval(tick, RECONNECT_POLL_MS);

  // Coming back to the app is the moment this is for, so it does not wait for the next tick — and
  // it is a fresh instruction from the user, which earns the attempt count a reset.
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) return;

    // Coming back is the user asking again, which earns a fresh budget. A reload cannot reach this:
    // it replaces the document rather than hiding it, so the loop above cannot refill its own tank.
    attempts = 0;
    nextAttemptAt = 0;
    formSince = 0;

    forgetAttempts();
    tick();
  });

  // Deliberately does not reset the budget: this fires on its own, and a reconnect loop that keeps
  // refilling itself is the thing being prevented.
  window.addEventListener('online', tick);
}

const ATTEMPTS_KEY = 'shiver-reconnect-attempts';

/** How many times Shiver has already tried to get this page connected, across any reloads it did. */
function readAttempts(): number {
  try {
    return Number(sessionStorage.getItem(ATTEMPTS_KEY)) || 0;
  } catch {
    return 0;
  }
}

function rememberAttempts(attempts: number) {
  try {
    sessionStorage.setItem(ATTEMPTS_KEY, String(attempts));
  } catch {
    // a page that denies storage simply gets the in-memory budget
  }
}

function forgetAttempts() {
  try {
    sessionStorage.removeItem(ATTEMPTS_KEY);
  } catch {
    // as above
  }
}

/**
 * Whether this server is answering at all.
 *
 * `navigator.onLine` is not usable for this — measured on Android with both wifi and mobile data
 * turned off, it still reports `true`. So Shiver asks the server itself, on the endpoint it already
 * uses to identify one.
 */
function reachable(): Promise<boolean> {
  return fetch(`${location.origin}/info`, { method: 'GET', cache: 'no-store' })
    .then((response) => response.ok)
    .catch(() => false);
}

/** Covers the server's "connection lost" screen with Shiver saying what is actually happening. */
function showReconnecting() {
  if (document.getElementById(RECONNECT_HOST_ID)) return;

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const style = document.createElement('style');
  const panel = document.createElement('div');
  const spinner = document.createElement('div');
  const label = document.createElement('p');
  const server = shiverConfig?.serverName;

  host.id = RECONNECT_HOST_ID;

  // below the rail, which stays reachable: a server that will not come back is a good reason to
  // want the one next to it
  style.textContent = `
:host { position: fixed; inset: 0; z-index: 2147483645; }
.panel { position: absolute; inset: 0; display: flex; flex-direction: column;
  align-items: center; justify-content: center; gap: 18px; background: #0a0a0a;
  font: 500 15px/1.3 system-ui, -apple-system, "Segoe UI", sans-serif; color: #a1a1a1; }
.spinner { width: 34px; height: 34px; border-radius: 50%;
  border: 3px solid rgb(255 255 255 / 14%); border-top-color: #e5e5e5;
  animation: spin 900ms linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 3s; } }
`;

  panel.className = 'panel';
  spinner.className = 'spinner';
  label.textContent = server ? `Reconnecting to ${server}…` : 'Reconnecting…';

  panel.append(spinner, label);
  root.append(style, panel);
  document.body.append(host);
}

function hideReconnecting() {
  document.getElementById(RECONNECT_HOST_ID)?.remove();
}

/**
 * Takes the words off Sharkord's loading screen.
 *
 * Shiver now signs the user in by seeding that server's own auto-login, so every switch between
 * servers passes through its "Logging in automatically…" screen — an accurate description of what
 * Sharkord is doing, and a strange thing to read when what you did was tap a server in a rail. Shiver
 * has already said "Connecting to <server>" on its own boot screen a moment earlier; this is the
 * same wait described twice, in someone else's words.
 *
 * The spinner stays. Hiding the whole screen would leave a black gap that reads as a hang, and the
 * spinner is what makes the two loading states look like one.
 *
 * Matched on the container's full class list rather than a test id, because that screen carries no
 * test id. If Sharkord restyles it the rule simply stops matching and the text comes back — which is
 * where this started, so the failure is the old behaviour rather than a broken client.
 */
function quietTheLoadingScreen() {
  ensureStyle(QUIET_STYLE_ID).textContent = `
div.flex.flex-col.justify-center.items-center.h-full.gap-2 > span.text-xl { display: none !important; }
`;
}

/**
 * Hands this server's own client the session Shiver holds for it.
 *
 * Shiver signed in at the add screen, so the user should not meet a connect form on the way to a
 * server they already gave a password for. Rather than reimplement Sharkord's sign-in, this writes
 * the two things its own "Login automatically" writes and lets its `AutoLoginController` do the
 * work — the mechanism the server already has, driven with a token it already issued.
 *
 * It runs at page load, which is before that controller decides anything: the controller waits on
 * the app and its plugins to finish loading. If it ever lost that race the user would simply see
 * the connect form, which is where they were before Shiver could sign in at all.
 *
 * Existing state is left alone. Someone who signed out on the page meant it, and this must not sign
 * them back in.
 */
/**
 * Puts back what Shiver kept when it last cleared this server's storage.
 *
 * Dropping the origin's storage is what keeps a session off the disk, but it takes the user's own
 * things with it — the channel they were reading, and anything typed and not sent. Those are
 * carried across in Shiver's encrypted store and written back here, before Sharkord's own code looks
 * for them.
 *
 * Never overwrites. Anything already in the page is newer than a value from a previous visit, so
 * this only fills gaps.
 */
function restoreCarried(carried: string) {
  try {
    const state = JSON.parse(carried) as Record<string, unknown> | null;

    if (!state || typeof state !== 'object') return;

    // Only what is missing. The page has already started by the time this runs, and a value it has
    // written since is newer than anything kept from last time.
    for (const [key, value] of Object.entries(state)) {
      if (typeof value !== 'string') continue;
      // never put a session back: Shiver holds that, and `forget_page_session` cleared it on purpose
      if (key === 'sharkord-identity' || key.startsWith('sharkord-auto-login')) continue;
      if (localStorage.getItem(key) !== null) continue;

      localStorage.setItem(key, value);
    }
  } catch {
    // unreadable carried state is not worth failing a page load over
  }
}

function seedSession(session: string) {
  try {
    // Cleared first, not skipped-if-present. This runs on arrival, so it also clears a token left
    // behind by a previous visit that ended abnormally — a crash, or the app being killed while the
    // server was open — which is the one case the clean-up on leave cannot cover.
    localStorage.removeItem('sharkord-auto-login');
    localStorage.removeItem('sharkord-auto-login-token');

    localStorage.setItem('sharkord-auto-login', 'true');
    localStorage.setItem('sharkord-auto-login-token', session);
    sessionStorage.setItem('sharkord-token', session);

    // remembered so the clean-up only ever removes Shiver's own doing
    window.__SHIVER_SEEDED__ = true;
  } catch {
    // a page that denies storage signs in the ordinary way
  }
}

/**
 * Takes Shiver's sign-in back out of the page on the way out of a server.
 *
 * The token Shiver holds lives encrypted, in the Android Keystore's care. Seeding it into the page
 * writes that same token into the webview's `localStorage`, which is a plain file on disk — so
 * leaving it there would quietly undo the encrypted store for anyone who can read the app's data.
 * Shiver puts it in to sign the user in and takes it out again when the server is no longer on screen,
 * so the window it exists in is the window it is being used in.
 *
 * Only what Shiver seeded. A user who ticked "Login automatically" on the server's own page meant it,
 * and that is not Shiver's to delete — hence the flag rather than clearing the keys unconditionally.
 *
 * On the way out rather than as soon as the client connects: Sharkord's auto-login controller runs
 * again if the socket drops, so a token removed while the page is still up would turn a recoverable
 * disconnect into a login form.
 */
function forgetSession() {
  if (!window.__SHIVER_SEEDED__) return;

  try {
    localStorage.removeItem('sharkord-auto-login');
    localStorage.removeItem('sharkord-auto-login-token');
    sessionStorage.removeItem('sharkord-token');
  } catch {
    // nothing to take back out of a page that denied storage in the first place
  }

  window.__SHIVER_SEEDED__ = false;
}

/**
 * The menu actions that only this server's own client can carry out.
 *
 * Exposed for the core to call rather than done from the menu directly, so both rails route the
 * same action through the same code — Shiver's own page has no client to drive and asks the core,
 * which asks this.
 */
function installPageActions() {
  window.__SHIVER_MARK_ALL_READ__ = markAllChannelsRead;
  window.__SHIVER_SIGN_OUT__ = signOut;
  window.__SHIVER_FORGET_SESSION__ = forgetSession;
}

/**
 * Marks every text channel read, the way desktop's bridge does it.
 *
 * Sharkord marks a channel read when it is selected, and there is no api for "all of them", so
 * Shiver selects each in turn and puts the user back where they were. A channel with nothing unread
 * returns early inside Sharkord, so this costs it no requests.
 */
function markAllChannelsRead() {
  const store = sharkordStore();
  const selectChannel = store?.actions?.selectChannel;

  if (!selectChannel) return;

  const state = store?.getState();
  const previous = state?.selectedChannelId;
  const channels = (state?.channels ?? []).filter(
    (channel) => !channel.isDm && channel.type === 'TEXT'
  );

  for (const channel of channels) selectChannel(channel.id);

  if (typeof previous === 'number') selectChannel(previous);
}

/**
 * Ends the session this page holds.
 *
 * Shiver stores no credentials on Android, so there is nothing of its own to forget — signing out
 * means clearing what *Sharkord* persisted: the auto-login token that would otherwise sign the user
 * straight back in, the live session, and the remembered identity. The reload then lands on the
 * server's own connect form, which is what logging out should look like.
 */
function signOut() {
  for (const key of ['sharkord-auto-login', 'sharkord-auto-login-token', 'sharkord-identity']) {
    try {
      localStorage.removeItem(key);
    } catch {
      // a page that denies storage has nothing to clear
    }
  }

  try {
    sessionStorage.removeItem('sharkord-token');
  } catch {
    // as above
  }

  window.location.reload();
}

type SharkordChannel = { id: number; name: string; isDm?: boolean; type?: string };
type SharkordRole = { id: number; name: string; color?: string; isDefault?: boolean };
type SharkordUser = { id: number; name: string; roleIds?: number[] };

/** The plugin store, which is the only way into Sharkord's own state. */
function sharkordStore() {
  return (
    window as unknown as {
      __SHARKORD_STORE__?: {
        getState: () => {
          channels?: SharkordChannel[];
          users?: SharkordUser[];
          roles?: SharkordRole[];
          ownUserId?: number;
          selectedChannelId?: number;
        };
        subscribe?: (listener: () => void) => () => void;
        actions?: { selectChannel?: (id: number) => void };
      };
    }
  ).__SHARKORD_STORE__;
}

/**
 * Whether Sharkord's channel drawer is on screen.
 *
 * Measured rather than read off a class: Sharkord slides it with Tailwind's `-translate-x-full`,
 * and matching on class names would break the moment its layout changed. Above `md` the sidebar is
 * in normal flow and always showing, which this reports as open — correctly, because there is no
 * first swipe to make on a screen that wide, and the first one should be Shiver's.
 */
function drawerIsOpen() {
  const sidebar = document.querySelector(SIDEBAR);

  if (!(sidebar instanceof HTMLElement)) return window.innerWidth >= WIDE_LAYOUT;

  return sidebar.getBoundingClientRect().right > 1;
}

/** Whether this page is a Sharkord client view at all, rather than a sign-in or an error page. */
function hasSidebar() {
  return document.querySelector(SIDEBAR) instanceof HTMLElement;
}

/** Keeps Shiver out of horizontal gestures in the message area, which are not the rail's business. */
function startedNearTheDrawer(x: number) {
  const sidebar = document.querySelector(SIDEBAR);
  const right = sidebar instanceof HTMLElement ? sidebar.getBoundingClientRect().right : 0;

  return x <= Math.max(right, 0) + 40;
}

/* ───────────────────────── direct messages ───────────────────────── */

/**
 * Shiver's own list of conversations, across every server.
 *
 * Sharkord's list is per server and lives in that server's sidebar, so tapping the tile used to
 * show only the conversations on whichever server happened to be open. This one is drawn by Shiver,
 * from what the core collected over its own connections, and covers all of them.
 *
 * Deliberately shaped like the list it replaces — an initial, a name, the server underneath — so
 * the tab still reads as the same tab. The initial stands in for Sharkord's avatar: the picture
 * lives on that server, and fetching it from inside another server's page would tell that page
 * where the other server is, which is the one thing the rail never says.
 */
/**
 * This server's own conversations, read from the page.
 *
 * Shiver's core learns conversations from the servers it holds background connections to, and it
 * deliberately holds none for the server on screen — that server reports for itself. Which left the
 * one server the user is actually looking at contributing nothing to the list: with two servers and
 * conversations only on the one in front of you, the panel was empty and said so.
 *
 * Sharkord names a direct-message channel `DM - <a>:<b>` after the two people in it
 * (`routers/dms/open-direct-message.ts`), and the store carries `ownUserId` and the user list — so
 * the page has everything needed to say who each conversation is with, without a round trip.
 */
function pageDms(shiver: ShiverConfig): DirectMessage[] {
  const state = sharkordStore()?.getState();
  const ownUserId = state?.ownUserId;

  if (!state?.channels || !ownUserId) return [];

  const conversations = state.channels
    .filter((channel) => channel.isDm)
    .map((channel) => {
      const pair = /^DM - (\d+):(\d+)$/.exec(channel.name ?? '');

      if (!pair) return null;

      const partner = [Number(pair[1]), Number(pair[2])].find((id) => id !== ownUserId);

      return partner === undefined ? null : { channelId: channel.id, partner };
    })
    .filter((conversation): conversation is { channelId: number; partner: number } => !!conversation);

  // named from the user list, which on a big server is tens of thousands of people — so it is
  // walked once for the handful of ids this user is actually in a conversation with
  const wanted = new Set(conversations.map((conversation) => conversation.partner));
  const names = new Map<number, string>();

  for (const user of state.users ?? []) {
    if (wanted.has(user.id)) names.set(user.id, user.name);
  }

  return (
    conversations
      .map(({ channelId, partner }) => ({
        entryId: shiver.entryId,
        serverName: shiver.serverName,
        channelId,
        // a conversation with someone no longer in the user list still exists, and saying so is
        // better than dropping it without explanation — the same call the core makes
        userName: names.get(partner) ?? 'Unknown'
      }))
      // By name, which is not what Shiver's own screen does — that one orders by the latest message,
      // and this one cannot. `dms.get` is where Sharkord's `lastMessageAt` comes from, and its own
      // client keeps that answer in component state rather than in the plugin store, so nothing on
      // this page can read it. The store carries `channels`, whose order is roughly when each
      // conversation was created, and ordering by that would look arbitrary without being useful.
      //
      // So: alphabetical, which is at least stable and searchable by eye. The way to recency here
      // would be the core pushing this server's own timestamps into the page — its own data, so no
      // boundary problem — but they would be as stale as the last time Shiver held a connection to a
      // server it is now looking at, which is a worse answer than a predictable one.
      .sort((a, b) => a.userName.localeCompare(b.userName))
  );
}

/**
 * The panel showing this server's conversations, and the way to the rest.
 *
 * It used to list every server's, handed over by the core at page load. That told whichever server
 * the user happened to be looking at the name of everyone they privately message on every other
 * server they have added — so the cross-server list moved to Shiver's own screen, which a server's
 * page cannot read, and this panel now draws only what this page already knows.
 *
 * The common case costs nothing: `pageDms` reads this server's conversations out of Sharkord's own
 * store, so they appear instantly and without a round trip. Only the cross-server case navigates.
 */
function openDmPanel(root: ShadowRoot, shiver: ShiverConfig, closeRail: () => void) {
  root.querySelector('.dm-panel')?.remove();

  const panel = el('div', 'dm-panel');
  const heading = el('div', 'dm-heading');

  heading.textContent = 'Direct messages';
  panel.append(heading);

  const close = () => panel.remove();

  // this server's own, read live from the page — its data, already in reach, so no round trip
  const dms = pageDms(shiver);

  if (!dms.length) {
    const empty = el('p', 'dm-empty');

    // Said plainly rather than left blank, and it now says something narrower than it used to: this
    // panel is about this server, and the line below is the way to the others.
    empty.textContent = 'No conversations on this server yet.';
    panel.append(empty);
  }

  for (const dm of dms) {
    const row = el('button', 'dm-row');
    const avatar = el('span', 'dm-avatar');
    const text = el('span', 'dm-text');
    const name = el('span', 'dm-name');
    const where = el('span', 'dm-where');

    row.setAttribute('type', 'button');
    avatar.textContent = initial(dm.userName);
    name.textContent = dm.userName;
    where.textContent = dm.serverName;

    text.append(name, where);
    row.append(avatar, text);

    row.addEventListener('click', () => {
      close();
      closeRail();

      // already here: Sharkord's own list is the thing that knows how to open a conversation, so
      // Shiver asks it rather than trying to select the channel itself
      openConversation(dm.userName);
    });

    panel.append(row);
  }

  // The way to every other server's conversations, which live on Shiver's own screen because that is
  // the only place they can be listed without this page being able to read them.
  const elsewhere = el('button', 'dm-row dm-elsewhere');

  elsewhere.setAttribute('type', 'button');
  elsewhere.textContent = 'Conversations on your other servers';

  elsewhere.addEventListener('click', () => {
    close();
    closeRail();
    goHome('#dms');
  });

  panel.append(elsewhere);

  // a tap anywhere off the panel closes it, the rail's menus behave the same way
  const dismiss = (event: Event) => {
    if (event.target instanceof Node && panel.contains(event.target)) return;

    close();

    root.removeEventListener('click', dismiss, true);
  };

  root.append(panel);
  root.addEventListener('click', dismiss, true);
}

/**
 * Whether a row in Sharkord's list is the conversation Shiver is looking for.
 *
 * Not a plain comparison, because the row's text is not just the name: where the other person has
 * no picture, Sharkord's avatar falls back to their initial, and that letter is a text node inside
 * the row — so "Wanderer" arrives as "WWanderer". Measured on a real one, after a first attempt
 * matched nothing at all.
 */
function namesTheSamePerson(rowText: string, userName: string) {
  const text = rowText.trim();

  if (text === userName) return true;

  // only an avatar's worth of characters may sit in front of the name, so one person's name being
  // the tail of another's cannot match the wrong row
  return text.endsWith(userName) && text.length - userName.length <= 2;
}

/** Keeps the rail's own copy of where a server lives in step with what the core has been told. */
function entryFolder(shiver: ShiverConfig, serverId: string, folderId: string | null) {
  const entry = shiver.rail.find((candidate) => candidate.id === serverId);

  if (entry) entry.folderId = folderId;
}

/**
 * A name for a folder the rail has just made.
 *
 * `randomUUID` where the page has it — every server Shiver will open is https, so it usually does —
 * and something unique enough otherwise. It only has to not collide with an existing folder.
 */
function newFolderId() {
  return crypto.randomUUID
    ? crypto.randomUUID()
    : `rail-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** The letter shown where Sharkord would show an avatar. */
function initial(name: string) {
  return (name.trim()[0] ?? '?').toUpperCase();
}

/**
 * Lands on one conversation in Sharkord's own direct-message list.
 *
 * Matched on the name, because the rows carry nothing else to match on — no channel id reaches the
 * dom. Sharkord opens the conversation itself from there, which keeps Shiver out of a part of the
 * client it has no business reimplementing.
 */
function openConversation(userName: string) {
  openDirectMessages();

  whenPresent(DM_ITEM, () => {
    const rows = document.querySelectorAll<HTMLElement>(DM_ITEM);

    for (const row of rows) {
      if (namesTheSamePerson(row.textContent ?? '', userName)) {
        row.click();

        return;
      }
    }
  });
}

/**
 * Opens Sharkord's own direct-message list for this server.
 *
 * Shiver cannot show dms itself here. They are per server and live inside the client's own sidebar,
 * and with one webview there is no second surface to render them on — so the rail's dm tile asks
 * this server's client to do it, which is Shiver's whole posture: never reimplement Sharkord.
 *
 * The drawer is opened first by synthesizing the swipe Sharkord already listens for, because it
 * exposes no other way in. If that fails the toggle is still clicked, so the dm list is what the
 * user finds when they open the drawer themselves.
 */
function openDirectMessages() {
  // The bridge runs on page load, which on a single-page client is *before* its own app has
  // mounted — so on arrival there is nothing here yet to click. Waiting for the toggle to exist is
  // the difference between the dm tile working and doing nothing at all on a cold open.
  whenPresent(DM_TOGGLE, (toggle) => {
    if (!drawerIsOpen()) openDrawer();

    // after the drawer's own transition, so the list the user lands on is the one they asked for
    window.setTimeout(() => toggle.click(), 80);
  });
}

/** How long to wait for Sharkord's client to render before giving up on it. */
const APPEAR_TIMEOUT = 15_000;

/** Calls back with the first element matching `selector`, now or once the client renders it. */
function whenPresent(selector: string, use: (element: HTMLElement) => void) {
  const existing = document.querySelector(selector);

  if (existing instanceof HTMLElement) {
    use(existing);

    return;
  }

  const observer = new MutationObserver(() => {
    const found = document.querySelector(selector);

    if (!(found instanceof HTMLElement)) return;

    observer.disconnect();
    use(found);
  });

  observer.observe(document.body, { childList: true, subtree: true });

  // a server whose dm list never appears — disabled by its admin, or a sign-in page — must not
  // leave an observer running over every mutation for the life of the session
  window.setTimeout(() => observer.disconnect(), APPEAR_TIMEOUT);
}

/** Sharkord's drawer has no api, only a gesture, so Shiver performs the gesture. */
function openDrawer() {
  const view = document.querySelector('[data-testid="server-view"]') ?? document.body;

  try {
    const at = (clientX: number) => {
      const touch = new Touch({ identifier: 1, target: view, clientX, clientY: 300 });

      return { touches: [touch], changedTouches: [touch], bubbles: true } as TouchEventInit;
    };

    view.dispatchEvent(new TouchEvent('touchstart', at(8)));
    view.dispatchEvent(new TouchEvent('touchmove', at(200)));
    view.dispatchEvent(new TouchEvent('touchend', { ...at(200), touches: [] }));
  } catch {
    // no Touch constructor: the toggle click below still selects dms, the user opens the drawer
  }
}

/* ────────────────────────────── styling ────────────────────────────── */

function railStyle() {
  const style = document.createElement('style');

  // Sharkord's own dark palette, matching Shiver's desktop rail token for token, so the rail looks
  // like one component across the two clients.
  //
  // Every colour is a custom property with that palette as its fallback. Nothing is set while the
  // user keeps the default colours, so the fallbacks are what draws — and when they do pick
  // colours, `applyTheme` sets the properties on the document and they inherit in here, shadow root
  // and all, which is how the rail came to be the one surface their background never reached.
  style.textContent = `
:host { position: fixed; inset: 0; z-index: 2147483647; pointer-events: none;
  /* Sharkord's own drawer timing, matched by eye rather than measured: nothing has to stay in
     step with it any more, but the rail should still feel like part of the same client */
  --shiver-slide: 300ms; --shiver-ease: cubic-bezier(0.4, 0, 0.2, 1);
  --shiver-rail-width: ${RAIL_WIDTH}px;
  font: 600 14px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
:host(.open) .scrim { opacity: 1; pointer-events: auto; }
:host(.open) .rail { transform: translateX(0); }

.scrim { position: absolute; inset: 0; background: rgb(0 0 0 / 45%);
  opacity: 0; transition: opacity var(--shiver-slide) var(--shiver-ease); transition-delay: 0s; }

.rail { position: absolute; top: 0; bottom: 0; left: 0; width: var(--shiver-rail-width);
  box-sizing: border-box;
  /* The same 10px top and bottom. The top used to be the bare inset, which was the whole gap
     back when the webview drew under the status bar; now that the activity insets it and
     consumes the insets, that value is zero and the first tile sat flush against the bar.
     The env() stays for anywhere the platform still has something to report. */
  padding: calc(10px + env(safe-area-inset-top, 0px)) 0
    calc(10px + env(safe-area-inset-bottom, 0px));
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  background: var(--shiver-rail, #171717);
  border-right: 1px solid var(--shiver-border, rgb(255 255 255 / 10%));
  overflow-y: auto; overflow-x: hidden; scrollbar-width: none;
  transform: translateX(-100%); pointer-events: auto;
  transition: transform var(--shiver-slide) var(--shiver-ease); transition-delay: 0s; }
.rail::-webkit-scrollbar { display: none; }

.tile { position: relative; width: 48px; height: 48px; flex: 0 0 48px;
  border: none; padding: 0; border-radius: 16px;
  background: var(--shiver-surface, #262626); color: var(--shiver-text, #fafafa);
  display: grid; place-items: center; font: inherit; font-size: 16px;
  /* Nothing here is text to select, and a long press that starts a selection instead sends a
     touchcancel event, which took the drag apart before it could begin. */
  user-select: none; -webkit-user-select: none; -webkit-touch-callout: none;
  transition: border-radius 120ms ease, background 120ms ease; }
.tile:active { background: var(--shiver-surface-hover, #333333); border-radius: 14px; }
.tile.active { border-radius: 14px;
  box-shadow: inset 0 0 0 2px var(--shiver-accent, #e5e5e5); }
.tile.add { color: var(--shiver-text, #e5e5e5); font-size: 24px; }

/* Shut, a folder wears its contents: up to four of them in a 2x2 grid. The rows are stated as well
   as the columns on purpose — left to the count, one member stretches to fill the tile and reads as
   a server whose icon has been cut in half. */
.tile.tile-folder { background: var(--shiver-surface-dim, #1f1f1f); padding: 4px; }
.folder-grid { display: grid; grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr;
  gap: 2px; width: 100%; height: 100%; }
.folder-cell { border-radius: 5px;
  background: var(--shiver-surface-hover, #333333); color: var(--shiver-text, #e5e5e5);
  display: grid; place-items: center; font-size: 9px; font-weight: 700; overflow: hidden; }
.folder-cell img { width: 100%; height: 100%; object-fit: cover;
  pointer-events: none; -webkit-user-drag: none; }

/* Open, it shrinks to a marker and the servers appear below it. */
.tile.tile-folder.open { height: 20px; flex: 0 0 20px; border-radius: 10px; padding: 0; }
.folder-open { width: 16px; height: 3px; border-radius: 2px;
  background: var(--shiver-text-dim, #8a8a8a); }
/* No background of its own. Desktop can tint this box because its rail and its page are different
   shades; here the rail is already the darker surface, and a darker box around one tile reads as a
   black oval drawn round the icon rather than as a group. The indent and the spacing group them. */
.folder-contents { display: flex; flex-direction: column; align-items: center; gap: 8px;
  padding: 4px 0; }
.tile.in-folder { width: 40px; height: 40px; flex: 0 0 40px; border-radius: 13px;
  font-size: 14px; }
/* The icon is decoration; every touch belongs to the tile under it. */
.tile img { width: 100%; height: 100%; object-fit: cover; border-radius: inherit;
  pointer-events: none; -webkit-user-drag: none; }

/* Overhangs the tile on purpose, which is why the tile itself clips nothing — the icon rounds
   itself instead. The desktop rail's badge, to the pixel, hairline included. */
/* Pale with dark text, which is Sharkord's own unread badge rather than Shiver's invention — and it
   frees red to mean the offline mark, something wanted from you rather than merely unread. */
.badge { position: absolute; top: -2px; right: -2px; min-width: 16px; height: 16px;
  padding: 0 4px; border-radius: 8px; background: var(--shiver-text, #fafafa);
  color: var(--shiver-rail, #171717);
  font-size: 9px; line-height: 16px; font-weight: 700; pointer-events: none;
  box-shadow: 0 0 0 1px var(--shiver-rail, #171717); }
.tile svg { width: 16px; height: 16px; display: block; }

/* Sits beside the rail rather than over it, so the tile it belongs to stays visible — the same
   relationship a native context menu has to the thing it was opened from. */
.menu { position: absolute; left: calc(var(--shiver-rail-width) + 8px);
  min-width: 200px; padding: 6px; border-radius: 12px;
  border: 1px solid var(--shiver-border, rgb(255 255 255 / 12%));
  background: var(--shiver-surface-dim, #1f1f1f); color: var(--shiver-text, #fafafa);
  box-shadow: 0 12px 32px rgb(0 0 0 / 55%); pointer-events: auto; z-index: 1; }
.menu-item { display: block; width: 100%; padding: 11px 12px; border: none;
  border-radius: 8px; background: none; color: inherit; font: inherit; text-align: left; }
.menu-item:active { background: #333333; }

/* The direct-message list, sized and spaced like the one it stands in for. It sits beside the rail
   rather than over it, so the rail is still there to switch servers with. */
.dm-panel { position: absolute; top: 0; bottom: 0; left: var(--shiver-rail-width);
  width: min(288px, calc(100vw - var(--shiver-rail-width))); box-sizing: border-box;
  padding: calc(10px + env(safe-area-inset-top, 0px)) 8px
    calc(10px + env(safe-area-inset-bottom, 0px));
  background: var(--shiver-surface-dim, #1f1f1f);
  border-right: 1px solid var(--shiver-border, rgb(255 255 255 / 10%));
  overflow-y: auto; scrollbar-width: none; pointer-events: auto; z-index: 1; }
.dm-panel::-webkit-scrollbar { display: none; }
.dm-heading { padding: 8px 8px 12px; color: var(--shiver-text-dim, #a1a1a1); font-size: 12px;
  text-transform: uppercase; letter-spacing: 0.04em; }
.dm-empty { margin: 0; padding: 8px; color: var(--shiver-text-dim, #8a8a8a);
  font-size: 13px; line-height: 1.4;
  font-weight: 400; }
.dm-row { display: flex; width: 100%; align-items: center; gap: 8px; padding: 8px;
  border: none; border-radius: 8px; background: none; color: var(--shiver-text, #e5e5e5);
  font: inherit; text-align: left; }
.dm-row:active { background: var(--shiver-surface-hover, #333333); }
.dm-avatar { flex: 0 0 28px; width: 28px; height: 28px; border-radius: 50%;
  background: var(--shiver-surface-hover, #3a3a3a); color: var(--shiver-text, #e5e5e5);
  display: grid; place-items: center; font-size: 13px; }
.dm-text { display: flex; flex-direction: column; min-width: 0; }
.dm-name { font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dm-where { color: var(--shiver-text-dim, #8a8a8a); font-size: 11px; font-weight: 400;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* The hand-off to Shiver's own screen. Set apart above the rule, because it leaves this server
   rather than opening something on it. */
.dm-elsewhere { margin-top: 8px; padding-top: 14px; color: var(--shiver-text-dim, #a1a1a1);
  font-size: 13px; border-top: 1px solid var(--shiver-border, rgb(255 255 255 / 10%));
  border-radius: 0; }
.menu-divider { height: 1px; margin: 6px 4px;
  background: var(--shiver-border, rgb(255 255 255 / 12%)); }

/* A tile under the finger. Raised rather than dimmed, so it reads as picked up and the rail
   underneath stays legible while it is moved through. */
.tile.lifted { transform: scale(1.12); box-shadow: 0 8px 20px rgb(0 0 0 / 55%);
  opacity: 0.92; transition: transform 120ms ease; z-index: 2; }

/* A server Shiver can no longer reach on its own: its session expired and there is no password kept
   for it, so it reports nothing until the user signs in. Dimmed with a ring rather than badged,
   because this is an absence of news rather than news — and the ring is what says the silence has
   a reason. Opening the server, or signing in from Shiver's settings, clears it. */
.tile.signed-out { opacity: 0.55; box-shadow: 0 0 0 2px var(--shiver-danger, #ff6467); }

/* The tile the held one would join. Ringed rather than moved, because nothing has happened yet and
   a tile that shifts would read as a reorder. */
.tile.drop-onto { box-shadow: 0 0 0 2px var(--shiver-accent, #e5e5e5); }

.spacer { flex: 1 1 auto; min-height: 8px; }
.divider { width: 32px; height: 1px; flex: 0 0 1px;
  background: var(--shiver-border, rgb(255 255 255 / 10%)); }

/* No edge tab. The rail was given one so the swipe was not the only way in, and it earned its keep
   while the gesture was unreliable — but the swipe now opens the rail from any page, sign-in screens
   included, and the tab sat on top of the client Shiver is supposed to be showing. */
`;

  return style;
}

/**
 * Darkens muted channels in Sharkord's own channel list, and stops them announcing themselves.
 *
 * Matched on the visible channel name, because Sharkord's rows carry `data-testid="channel-item"`
 * and no channel id. Two channels sharing a name dim together; muting itself is keyed on the id.
 */
function paintMuted(muted: Set<number>) {
  ensureStyle(MUTE_STYLE_ID).textContent = `
.${MUTE_CLASS} { opacity: 0.45; }
.${MUTE_CLASS} ${UNREAD_COUNT} { display: none !important; }
/* The reaction you are part of. Sharkord says so with a one-pixel border and nothing else, which is
   easy to miss at a glance; this tints the whole pill in the accent instead. Only a colour — the
   size and spacing stay Sharkord's, so nothing beside it moves. */
${REACTED_PILL} {
  background-color: color-mix(in srgb, var(--primary) 22%, transparent) !important;
  border-color: var(--primary) !important;
}
`;

  // No early return on an empty set, deliberately: the loop below is also what *clears* the class,
  // so skipping it would leave a channel dimmed after the user unmuted it, with nothing to undo it
  // short of a reload.
  const names = mutedNames(muted);

  for (const row of document.querySelectorAll<HTMLElement>(CHANNEL_ITEM)) {
    row.classList.toggle(MUTE_CLASS, names.has(rowName(row)));
  }
}

/** The plugin store is the only place a channel id and its name appear together. */
function mutedNames(muted: Set<number>) {
  const channels = sharkordStore()?.getState().channels ?? [];

  return new Set(channels.filter((channel) => muted.has(channel.id)).map((channel) => channel.name));
}

/**
 * Repaints the server's own client in the user's colours.
 *
 * Sharkord renders its app under a hard-coded `dark` class, so Shiver's overrides have to target that
 * as well as `:root` and win on specificity rather than on order alone.
 */
function applyTheme(theme: ShiverTheme | null) {
  if (!theme) {
    ensureStyle(THEME_STYLE_ID).textContent = '';

    return;
  }

  const { themeColor, accentColor, textColor } = theme;
  const mixInto = (color: string) => (isLightColor(color) ? '#000' : '#fff');
  const lift = (percent: number) =>
    `color-mix(in srgb, ${themeColor} ${percent}%, ${mixInto(themeColor)})`;
  // The user's own text colour where they have chosen one, and otherwise whichever of black or
  // white their background can carry — which is what this always did.
  const foreground = textColor ?? (isLightColor(themeColor) ? '#171717' : '#fafafa');
  const dim = `color-mix(in srgb, ${foreground} 65%, ${themeColor})`;

  ensureStyle(THEME_STYLE_ID).textContent = `
:root, .dark {
  /* Shiver's own rail, drawn in this page. Custom properties inherit through its shadow root, which
     is the only way to reach it from out here. */
  --shiver-rail: ${themeColor};
  --shiver-surface: ${lift(84)};
  --shiver-surface-hover: ${lift(72)};
  --shiver-surface-dim: ${lift(92)};
  --shiver-text: ${foreground};
  --shiver-text-dim: ${dim};
  --shiver-accent: ${accentColor};
  --shiver-border: color-mix(in srgb, ${foreground} 12%, transparent);

  --background: ${themeColor} !important;
  --foreground: ${foreground} !important;
  --sidebar: ${lift(90)} !important;
  --sidebar-foreground: ${foreground} !important;
  --card: ${lift(90)} !important;
  --card-foreground: ${foreground} !important;
  --popover: ${lift(88)} !important;
  --popover-foreground: ${foreground} !important;
  --muted-foreground: ${dim} !important;
  --muted: ${lift(84)} !important;
  --secondary: ${lift(84)} !important;
  --accent: ${lift(80)} !important;
  --accent-foreground: ${foreground} !important;
  --sidebar-accent: ${lift(80)} !important;
  --input: ${lift(78)} !important;
  --border: color-mix(in srgb, ${foreground} 12%, transparent) !important;
  --sidebar-border: color-mix(in srgb, ${foreground} 12%, transparent) !important;
  --primary: ${accentColor} !important;
  --primary-foreground: ${isLightColor(accentColor) ? '#171717' : '#fafafa'} !important;
  --sidebar-primary: ${accentColor} !important;
  --ring: ${accentColor} !important;
  --sidebar-ring: ${accentColor} !important;
}
`;
}

/** Relative luminance, matching Shiver's own `theme.ts` so both sides pick the same foreground. */
function isLightColor(hex: string) {
  const value = hex.replace('#', '');

  if (value.length !== 6) return false;

  const channel = (offset: number) => {
    const srgb = parseInt(value.slice(offset, offset + 2), 16) / 255;

    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };

  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4) > 0.35;
}

/* ─────────────────────────── the status button ─────────────────────────── */

const STATUS_BUTTON_ID = 'shiver-status-button';
const STATUS_POPOVER_ID = 'shiver-status-popover';
const STATUS_BACKDROP_ID = 'shiver-status-backdrop';
const STATUS_STYLE_ID = 'shiver-status-style';

/** Sharkord's own settings gear, which this button sits beside. A documented test id. */
const SETTINGS_TRIGGER = '[data-testid="user-settings-trigger"]';

/**
 * A way to set your status without crossing the app to find it.
 *
 * The same button the desktop app puts beside Sharkord's settings gear, and here for the same
 * reason it is there: the status is the plugin's, but the only place to write one used to be a tab
 * inside the settings screen, which is a long way to go for a line of text.
 *
 * Phone-shaped rather than a copy of the desktop one. The panel it sits in is the drawer, which is
 * narrow and can be swiped away, so the popover is sized to the viewport, placed on whichever side
 * of the button has room, and taken down when the drawer that anchors it goes.
 *
 * Nothing happens on a server without the plugin — `waitForPlugin` answers false and no space is
 * taken by a control that could not work.
 */
function installStatusButton() {
  ensureStyle(STATUS_STYLE_ID).textContent = `
#${STATUS_BACKDROP_ID} {
  position: fixed; inset: 0; z-index: 2147483646; background: rgb(0 0 0 / 45%);
}
#${STATUS_POPOVER_ID} {
  position: fixed; z-index: 2147483647; padding: 14px;
  width: min(320px, calc(100vw - 32px)); box-sizing: border-box;
  border-radius: 12px; display: flex; flex-direction: column; gap: 10px;
  background: var(--popover, #1f1f1f); color: var(--popover-foreground, #fafafa);
  border: 1px solid var(--border, rgb(255 255 255 / 12%));
  box-shadow: 0 12px 32px rgb(0 0 0 / 55%);
  font: 500 14px/1.4 system-ui, -apple-system, "Segoe UI", sans-serif;
}
#${STATUS_POPOVER_ID} label { color: var(--muted-foreground, #a1a1a1); font-size: 12px; }
#${STATUS_POPOVER_ID} input {
  width: 100%; box-sizing: border-box; padding: 10px; border-radius: 8px;
  background: var(--input, rgb(255 255 255 / 6%)); color: inherit; font: inherit;
  border: 1px solid var(--border, rgb(255 255 255 / 14%)); outline: none;
}
#${STATUS_POPOVER_ID} .shiver-status-note { margin: 0; color: #f87171; font-size: 12px; }
#${STATUS_POPOVER_ID} .shiver-status-row { display: flex; gap: 10px; justify-content: flex-end; }
#${STATUS_POPOVER_ID} button {
  min-height: 40px; padding: 8px 16px; border-radius: 8px; border: none; font: inherit;
  background: var(--primary, #e5e5e5); color: var(--primary-foreground, #171717);
}
#${STATUS_POPOVER_ID} button.shiver-status-ghost {
  background: transparent; color: var(--muted-foreground, #a1a1a1);
}
`;

  /** undoes whatever the open popover put on the window, so nothing outlives it */
  let releasePopover: (() => void) | null = null;

  const closePopover = () => {
    releasePopover?.();
    releasePopover = null;

    document.getElementById(STATUS_POPOVER_ID)?.remove();
    document.getElementById(STATUS_BACKDROP_ID)?.remove();
  };

  const openPopover = () => {
    if (document.getElementById(STATUS_POPOVER_ID)) {
      closePopover();

      return;
    }

    const host = document.createElement('div');
    const label = document.createElement('label');
    const input = document.createElement('input');
    const note = document.createElement('p');
    const row = document.createElement('div');
    const clear = document.createElement('button');
    const save = document.createElement('button');

    host.id = STATUS_POPOVER_ID;
    label.textContent = 'Your status — shown beside your name while you are online';
    input.maxLength = 100;
    input.placeholder = 'What are you up to?';
    input.enterKeyHint = 'done';

    note.className = 'shiver-status-note';
    note.hidden = true;

    clear.textContent = 'Clear';
    clear.className = 'shiver-status-ghost';
    save.textContent = 'Save';

    row.className = 'shiver-status-row';
    row.append(clear, save);
    host.append(label, input, note, row);

    const backdrop = document.createElement('div');

    backdrop.id = STATUS_BACKDROP_ID;

    // Shown before the current status is known, rather than after: asking first meant a tap did
    // nothing at all until the server answered, which on a phone reads as a button that is broken.
    document.body.append(backdrop, host);

    /**
     * Centred on the part of the screen the phone can actually see.
     *
     * It used to be anchored above the button that opened it, which is where the desktop one still
     * sits — and on a phone that put it under the keyboard. The button lives in the drawer's user
     * control at the bottom of the screen, focusing the field raises the keyboard over the bottom
     * half, and the position had already been worked out against the taller viewport, so the whole
     * thing was off-screen by the time anyone could type into it.
     *
     * `visualViewport` is the one that shrinks when the keyboard opens, so centring on it keeps the
     * field above the keys; `window.innerHeight` does not always move, which is what made this look
     * correct in code and wrong in the hand. It is re-run on every viewport change because the
     * keyboard arrives *after* the popover does.
     */
    const place = () => {
      const viewport = window.visualViewport;
      const width = viewport?.width ?? window.innerWidth;
      const height = viewport?.height ?? window.innerHeight;
      const size = host.getBoundingClientRect();

      host.style.left = `${(viewport?.offsetLeft ?? 0) + Math.max(8, (width - size.width) / 2)}px`;
      host.style.top = `${(viewport?.offsetTop ?? 0) + Math.max(8, (height - size.height) / 2)}px`;
    };

    place();

    const viewport = window.visualViewport;

    viewport?.addEventListener('resize', place);
    viewport?.addEventListener('scroll', place);

    releasePopover = () => {
      viewport?.removeEventListener('resize', place);
      viewport?.removeEventListener('scroll', place);
    };

    input.focus();

    let typed = false;

    input.addEventListener('input', () => {
      typed = true;
    });

    void callPlugin('getOwnStatus').then((current) => {
      // a status typed while the answer was in flight is the user's, and outranks it
      if (!host.isConnected || typed) return;

      input.value = typeof current?.status === 'string' ? current.status : '';
      input.select();
    });

    const commit = async (value: string) => {
      save.disabled = true;
      clear.disabled = true;
      note.hidden = true;

      const result = await callPlugin('setStatus', { status: value });

      // The relay answers with the status the server stored, so this is an acknowledgement rather
      // than a hope. Closing before it arrived would hide a failure behind a tidy animation.
      if (!result || typeof result.status !== 'string') {
        note.textContent = 'Could not save that. The Shiver plugin may no longer be installed here.';
        note.hidden = false;
        save.disabled = false;
        clear.disabled = false;

        return;
      }

      closePopover();
    };

    save.addEventListener('click', () => void commit(input.value));
    clear.addEventListener('click', () => void commit(''));

    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') void commit(input.value);
      if (event.key === 'Escape') closePopover();
    });

    // A tap outside closes it, the way Sharkord's own sheets behave — on the backdrop rather than
    // on the document, so the tap never reaches the page underneath. That matters: the drawer this
    // was opened from closes on an outside tap, and a tap that closed the drawer used to take the
    // popover with it, halfway through typing.
    backdrop.addEventListener('pointerdown', (event) => {
      event.preventDefault();
      closePopover();
    });
  };

  const add = () => {
    // the id lookup first, and deliberately: the observer below fires on every dom change in a busy
    // channel, and this is a hash lookup where the selector is a walk
    if (document.getElementById(STATUS_BUTTON_ID)) return;

    const gear = document.querySelector(SETTINGS_TRIGGER);

    if (!gear) return;

    const button = document.createElement('button');

    button.id = STATUS_BUTTON_ID;
    button.type = 'button';
    // borrowed from the gear beside it, so it matches whatever Sharkord's buttons look like today
    button.className = gear.className;
    button.title = 'Set your status';
    button.setAttribute('aria-label', 'Set your status');
    // lucide's `smile`, drawn inline because the bridge ships no assets
    button.innerHTML =
      '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24"' +
      ' fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"' +
      ' stroke-linejoin="round"><circle cx="12" cy="12" r="10"/>' +
      '<path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" x2="9.01" y1="9" y2="9"/>' +
      '<line x1="15" x2="15.01" y1="9" y2="9"/></svg>';

    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();

      openPopover();
    });

    gear.parentElement?.insertBefore(button, gear);
  };

  // The drawer is mounted and unmounted as it opens and closes, so adding the button once is not
  // enough. `add` is cheap and returns immediately when the button is already there.
  void waitForPlugin().then((present) => {
    if (!present) return;

    add();

    new MutationObserver(add).observe(document.body, { childList: true, subtree: true });
  });
}

/** Sharkord's file card: an anchor to the file, holding its icon, name, size and sometimes a bin. */
const FILE_CARD = 'a[class~="max-w-sm"][class~="rounded-lg"][class~="border-border"]';
const ATTACHMENT_STYLE_ID = 'shiver-attachment-cards';
const ATTACHMENT_MINIMISED = 'shiver-attachment-min';

/**
 * Shrinks the card under a picture down to its icon.
 *
 * Sharkord renders an image attachment **twice**: once as the picture, through `Media`, and again
 * as a file card underneath it with the filename and the size — see `renderer/index.tsx`, where
 * `<Media>` and the `message.files` list are siblings. On a channel where people post pictures that
 * second copy is most of what is on screen, and it says nothing the picture does not.
 *
 * Only the duplicates are touched, and duplicate is decided by the file's own address rather than
 * by its extension: a card is minimised when something on the page is already *showing* what it
 * points at. A pdf, a zip, a .txt — anything the client cannot render inline — keeps its card in
 * full, because there the card is the only thing there is.
 *
 * Minimised rather than hidden, and that is the point of the name. The card is also the delete
 * button on your own attachments and the download link on everyone else's, and a setting called
 * "tidier" that quietly takes the only way to remove a file with it is not tidier. What is left is
 * the icon and the bin, with the name and the size moved into the tooltip.
 */
function installAttachmentCards(minimised: boolean) {
  // Mobile runs `install` again on every page load in the same document, and a second observer
  // here would be a second one for ever. A repeat install is the setting changing, nothing more.
  const installed = window.__SHIVER_SET_ATTACHMENT_CARDS__;

  if (installed) {
    installed(minimised);

    return;
  }

  const style = ensureStyle(ATTACHMENT_STYLE_ID);

  style.textContent = `
a.${ATTACHMENT_MINIMISED} { max-width: none; width: fit-content; gap: 4px; padding: 2px;
  border-color: transparent; background: none; box-shadow: none; opacity: 0.55; }
a.${ATTACHMENT_MINIMISED}:hover { opacity: 1; }
/* the tile behind the icon, which is a frame around a frame at this size */
a.${ATTACHMENT_MINIMISED} > [class~="bg-muted"] { padding: 2px; background: none; }
/* the name and the size: both already above, in the picture and in its own filename */
a.${ATTACHMENT_MINIMISED} > [class~="flex-1"] { display: none; }
`;

  let on = minimised;

  const paint = () => {
    // Everything the page is currently showing, by address. Page-wide rather than per message:
    // the same picture posted twice is a duplicate in both places, and walking up to a message
    // container means naming one, which is a class name away from breaking.
    const shown = new Set<string>();

    for (const media of document.querySelectorAll<HTMLMediaElement | HTMLImageElement>(
      'img[src], video[src], audio[src], source[src]'
    )) {
      shown.add(media.src);
    }

    for (const card of document.querySelectorAll<HTMLAnchorElement>(FILE_CARD)) {
      const duplicate = on && shown.has(card.href);

      if (duplicate === card.classList.contains(ATTACHMENT_MINIMISED)) continue;

      card.classList.toggle(ATTACHMENT_MINIMISED, duplicate);

      // the name and size, kept reachable rather than thrown away. read from the card itself, so
      // it stays whatever Sharkord decided to call the file
      if (duplicate) {
        card.title = [...card.querySelectorAll('span')].map((span) => span.textContent?.trim()).join(' · ');
      } else {
        card.removeAttribute('title');
      }
    }
  };

  paint();

  new MutationObserver(paint).observe(document.body, { childList: true, subtree: true });

  window.__SHIVER_SET_ATTACHMENT_CARDS__ = (next: boolean) => {
    on = next;
    paint();
  };
}

const VOICE_COLORS_ID = 'shiver-voice-colors';

/**
 * Makes an active screen share purple in the voice bar, matching the member list.
 *
 * Sharkord disagrees with itself. `left-sidebar/voice-user.tsx` draws someone else's share with
 * `text-purple-500` and their camera with `text-blue-500`; `left-sidebar/voice-control.tsx` draws
 * *your own* active share with `bg-blue-500/15 … text-blue-400`. Sharing your screen therefore lights
 * a blue button while the row above shows a purple icon for the same thing — and blue already means
 * camera two inches away.
 *
 * Purple wins because the member list is where the ambiguity would be fatal: make the share blue
 * there and it becomes indistinguishable from the camera beside it. The button moves instead.
 *
 * `bg-blue-500/15` appears exactly once in the whole client, on that one button, so this needs no
 * further scoping. If a Sharkord update changes the class the rule stops matching and the button
 * goes back to blue, which is a thing you can see rather than a thing that fails quietly.
 *
 * The camera moves the same way, for the same reason and to the opposite conclusion: it is
 * `text-blue-500` in the member list and `text-blue-400` on the external-stream cards, and green
 * only on this one button. Blue wins on a count of one against three, and the two now read as a
 * pair — blue is the camera, purple is the screen, everywhere.
 *
 * The blue it takes is the exact blue the share button used to have, which is free now that the
 * share is purple.
 *
 * **Both controls exist twice**, in two components with different weights: the sidebar's voice bar
 * (`left-sidebar/voice-control.tsx`, /15 backgrounds and text-400) and the bar along the bottom of
 * a call (`channel-view/voice/controls-bar.tsx`, /20 and text-500). Recolouring one and not the
 * other is worse than recolouring neither, because then the same two buttons disagree with
 * themselves depending on where you are looking.
 *
 * Green stays wherever green means something else, and twice it does: the voice-debug chip in
 * `dialogs/voice-debug/parts.tsx` (healthy) and the pulsing ring on an external stream card
 * (live). Both share a background class with a camera button and neither has the matching hover
 * class, which is what the second selector in each pair is for.
 */
function installVoiceColors() {
  ensureStyle(VOICE_COLORS_ID).textContent = `
/* sharing your screen: purple, as the member list has always drawn it */
[class~="bg-blue-500/15"] {
  background-color: rgb(168 85 247 / 15%) !important;
  color: rgb(192 132 252) !important;
}
[class~="bg-blue-500/15"]:hover {
  background-color: rgb(168 85 247 / 25%) !important;
  color: rgb(216 180 254) !important;
}

/* your camera: blue, as every other place that draws a camera already does. the second class is
   what keeps the voice-debug "ok" chip green — it shares the first one and has no hover pair. */
[class~="bg-green-500/15"][class~="hover:bg-green-500/25"] {
  background-color: rgb(59 130 246 / 15%) !important;
  color: rgb(96 165 250) !important;
}
[class~="bg-green-500/15"][class~="hover:bg-green-500/25"]:hover {
  background-color: rgb(59 130 246 / 25%) !important;
  color: rgb(147 197 253) !important;
}

/* The same two controls again, in the bar along the bottom of a call. Separate rules because it is
   a separate component with its own weights — /20 backgrounds and text-500 rather than /15 and
   text-400 — and matching those is what keeps this looking like Sharkord rather than like a patch. */
[class~="bg-blue-500/20"] {
  background-color: rgb(168 85 247 / 20%) !important;
  color: rgb(168 85 247) !important;
}
[class~="bg-blue-500/20"]:hover {
  background-color: rgb(168 85 247 / 30%) !important;
  color: rgb(168 85 247) !important;
}

/* and again the hover class is the discriminator: bg-green-500/20 is also the pulsing ring on an
   external stream card, where green means live and has to stay. no backticks in here — this is a
   template literal, and one inside a comment ends the string. */
[class~="bg-green-500/20"][class~="hover:bg-green-500/30"] {
  background-color: rgb(59 130 246 / 20%) !important;
  color: rgb(59 130 246) !important;
}
[class~="bg-green-500/20"][class~="hover:bg-green-500/30"]:hover {
  background-color: rgb(59 130 246 / 30%) !important;
  color: rgb(59 130 246) !important;
}
`;
}

function ensureStyle(id: string) {
  const existing = document.getElementById(id);

  if (existing instanceof HTMLStyleElement) return existing;

  const style = document.createElement('style');

  style.id = id;
  document.documentElement.appendChild(style);

  return style;
}

/* ───────────────────────────── small parts ───────────────────────────── */

function el(tag: string, className: string) {
  const node = document.createElement(tag);

  node.className = className;

  return node;
}

function text(value: string) {
  return document.createTextNode(value);
}

function image(src: string) {
  const img = document.createElement('img');

  img.src = src;
  img.alt = '';
  // An image is draggable by default, and a long press on one starts the platform's own image drag
  // — which arrives as a touchcancel and takes Shiver's drag apart with it. It is why a server with a
  // custom icon could not be reordered while one showing its initials could. The desktop rail sets
  // the same attribute, for the same reason.
  img.draggable = false;

  return img;
}

/** Matches the desktop rail's fallback, so a server without a logo reads the same in both. */
function initials(name: string) {
  return (
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0]?.toUpperCase() ?? '')
      .join('') || '?'
  );
}

function svg(paths: string) {
  const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');

  node.setAttribute('viewBox', '0 0 24 24');
  node.setAttribute('fill', 'none');
  node.setAttribute('stroke', 'currentColor');
  node.setAttribute('stroke-width', '2');
  node.setAttribute('stroke-linecap', 'round');
  node.setAttribute('stroke-linejoin', 'round');
  node.innerHTML = paths;

  return node;
}

const messagesIcon = () =>
  svg('<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />');

const plusIcon = () => svg('<path d="M12 5v14M5 12h14" />');

const settingsIcon = () =>
  svg(
    '<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1" />'
  );

const config = window.__SHIVER__;

// Moved off `window` as soon as it has been read, so the session token it carries stops being
// readable from a global by anything else running in this page. See `shiverConfig` at the top.
delete window.__SHIVER__;

shiverConfig = config ?? null;

if (config) {
  try {
    install(config);
  } catch (error) {
    // never let a bridge failure take the user's client down with it
    console.error('[shiver] bridge failed to install', error);
  }
}

export {};
