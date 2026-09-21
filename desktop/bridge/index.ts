/**
 * The Shiver bridge.
 *
 * Runs as an initialization script inside every Sharkord page, before the page's own scripts. It is
 * the only Shiver code that ever executes on a server's origin, so it is handed this entry's config
 * and nothing else: no other server's name, messages or settings are reachable from here.
 *
 * It cannot call Shiver either. Sharkord pages have no Tauri IPC, so the bridge queues what it sees
 * and the core drains the queue with `__SHIVER_DRAIN__`.
 *
 * It reads Sharkord rather than driving it, with two deliberate exceptions: opening a specific DM
 * clicks Sharkord's own controls, and the channel context menu gains one extra item. Both use the
 * page's own affordances rather than reaching into its state.
 */

type ShiverTheme = {
  themeColor: string;
  accentColor: string;
  /** the text colour the user picked, or null to take it from the background */
  textColor: string | null;
};

type ShiverConfig = {
  entryId: string;
  origin: string;
  /** absent while the user is on Sharkord's own colours, and then nothing is restyled */
  theme: ShiverTheme | null;
  /** the session Shiver signed in with, seeded so the user never meets the login page */
  token: string | null;
  muted: number[];
  /**
   * `server` is the page the user browses channels in. `dm` is a second page for the same server
   * that only ever shows one conversation: it hides its own sidebar and reports nothing, because
   * the `server` page is already reporting and two would double every notification.
   */
  role: 'server' | 'dm';
  /**
   * For a `dm` page, the conversation to open as soon as it is able.
   *
   * It arrives in the config rather than by a call, because Shiver asks for a conversation at the
   * moment it creates the webview: the page has not loaded and the bridge does not exist yet, so an
   * eval at that point is dropped on the floor and the first open silently did nothing.
   */
  openDm: string | null;
  /**
   * Another server already holds the user's voice session, so joining one here is refused.
   *
   * It is a bare boolean on purpose: the page is never told *which* server holds it, so a server
   * learns only that the user is in a call somewhere, never anything identifying about the others.
   */
  voiceLocked: boolean;
  /**
   * Shrink the file card under a picture down to its icon.
   *
   * Sharkord draws an image attachment twice — the picture, and a card below it with the filename
   * and the size. This is whether the second one is worth the room it takes.
   */
  minimiseAttachments: boolean;
  /**
   * How loud this page's own sounds should be, as a percentage. 100 is Sharkord's own level.
   *
   * Above 100 is the point of it: an `<audio>` element's volume stops at 1.0, but every sound
   * either program makes is synthesised through Web Audio, and a gain node has no such ceiling.
   */
  soundVolume: number;
};

type QueuedNotification = {
  channelId: number | null;
  channelName: string | null;
  author: string;
  body: string;
  iconUrl: string | null;
  isDm: boolean;
};

type QueuedMute = {
  channelId: number;
  muted: boolean;
};

type PluginResponse = {
  source: string;
  id: string;
  ok: boolean;
  /** the union of what the relay's named actions answer with; each one returns its own part */
  result?: { mutedChannels?: number[]; status?: string };
  error?: string;
};

type DmChannel = {
  channelId: number;
  name: string;
  iconUrl: string | null;
  lastMessageAt: number | null;
};

/**
 * The user's voice session on *this* server, as far as this page can see it.
 *
 * The channel comes from the plugin store, which publishes `currentVoiceChannelId`. The mute flags
 * do not: `ownVoiceState` lives in the client's own redux store and is not exposed, so they are
 * read back off Sharkord's own controls instead.
 */
type VoiceSnapshot = {
  channelId: number;
  channelName: string | null;
  micMuted: boolean;
  soundMuted: boolean;
  /** Sharkord disables its own mic button while deafened, and Shiver's has to agree */
  micLocked: boolean;
};

type SharkordFile = {
  name: string;
  _accessToken?: string;
  _accessTokenExpiresAt?: number;
};

type SharkordChannel = {
  id: number;
  name: string;
  isDm?: boolean;
  /** `TEXT` or `VOICE` (`ChannelType` in `@sharkord/shared`) */
  type?: string;
  categoryId?: number | null;
  position?: number;
};

type SharkordCategory = {
  id: number;
  name: string;
  position?: number;
};

type SharkordUser = {
  id: number;
  name: string;
  avatar?: SharkordFile | null;
  roleIds?: number[];
};

type SharkordRole = { id: number; name: string; color?: string; isDefault?: boolean };

type SharkordState = {
  channels?: SharkordChannel[];
  categories?: SharkordCategory[];
  users?: SharkordUser[];
  roles?: SharkordRole[];
  ownUserId?: number;
  selectedChannelId?: number;
  currentVoiceChannelId?: number | null;
};

declare global {
  interface Window {
    __SHIVER__?: ShiverConfig;
    __SHIVER_DRAIN__?: () => {
      notifications: QueuedNotification[];
      dms: DmChannel[] | null;
      mutes: QueuedMute[];
      /** the plugin's copy of the mute list, sent once after it has been reconciled */
      syncedMutes: number[] | null;
      /** name of a conversation Shiver asked for and this page could not open */
      openDmFailed: string | null;
      /** the page has something for the user, so Shiver can take its loading circle down */
      ready: boolean;
      /** the client gave up on the seeded session and is asking for credentials */
      signedOut: boolean;
      /** the voice session on this server, or null when the user is not in one */
      voice: VoiceSnapshot | null;
      /** the channel on screen, so Shiver can treat looking at one as reading it */
      viewingChannelId: number | null;
      /** addresses the page tried to open in a new window, which is Shiver's to hand to the browser */
      open: string[];
      /**
       * Something in the page is filling the screen — a stream being watched, most often.
       *
       * Shiver's rail and bell are separate webviews floating over this one, so the page going
       * fullscreen does not move them: they stay on top of the thing that is supposed to be
       * filling the screen. The core takes them away while this is true.
       */
      fullscreen: boolean;
    };
    __SHIVER_SET_MUTED__?: (muted: number[]) => void;
    __SHIVER_OPEN_DM__?: (name: string) => void;
    /** jumps the page to one channel, for a notification the user clicked in Shiver's feed */
    __SHIVER_SELECT_CHANNEL__?: (channelId: number) => void;
    /** stores this user's unread floor for this server, so their devices share one badge */
    __SHIVER_SET_READ_FLOOR__?: (floor: Record<string, number>) => void;
    __SHIVER_SET_DM_MODE__?: (enabled: boolean) => void;
    __SHIVER_SET_THEME__?: (theme: ShiverTheme | null) => void;
    __SHIVER_SET_VOICE_LOCK__?: (locked: boolean) => void;
    /** turns the minimised attachment cards on or off without reloading the page */
    __SHIVER_SET_ATTACHMENT_CARDS__?: (minimised: boolean) => void;
    /** changes how loud this page's own sounds are without reloading it */
    __SHIVER_SET_SOUND_VOLUME__?: (percent: number) => void;
    /** drives Sharkord's own voice controls on Shiver's behalf */
    __SHIVER_VOICE__?: (action: 'mic' | 'sound' | 'leave') => void;
    /** marks every text channel on this server read, for the rail's context menu */
    __SHIVER_MARK_ALL_READ__?: () => void;
    /** set by the Shiver companion plugin's client bundle when it is installed on this server */
    __SHIVER_PLUGIN__?: { version: number };
    __SHARKORD_STORE__?: {
      getState: () => SharkordState;
      subscribe: (listener: () => void) => () => void;
      /**
       * Sharkord's own plugin-facing actions. Shiver uses `selectChannel` to mark channels read,
       * and the user-data pair to keep one unread floor across a person's devices.
       *
       * These take the plugin id as an argument, unlike `executePluginAction`, which reads it off
       * the calling stack — which is why those two can be called from here and it cannot.
       */
      actions?: {
        selectChannel?: (channelId: number) => void;
        getUserData?: (pluginId: string) => Promise<Record<string, unknown> | null>;
        setUserData?: (pluginId: string, data: Record<string, unknown>) => Promise<void>;
      };
    };
  }
}

/**
 * Defines one of Shiver's hooks so the page cannot replace it.
 *
 * These are ordinary `window` properties, and the core acts on whatever they return: the drain
 * feeds the notification inbox and the mute list, and one of its fields makes Shiver's own panel
 * ask the user for this server's password. A page that reassigns one is a page writing directly
 * into Shiver's trusted chrome.
 *
 * Non-writable and non-configurable, so a later assignment throws in strict mode and is ignored
 * otherwise, rather than quietly winning.
 */
/**
 * Runs `callback` once the page's DOM has settled, however many changes caused it.
 *
 * **One observer for the whole bridge, and one call per frame.** Each of these used to be its own
 * `MutationObserver` watching `document.body` with `subtree: true`, unthrottled — and in a chat
 * client the DOM mutates on every arriving message, every scroll of a virtualised list and every
 * typing indicator. Each callback then swept the document: the role-colour pass runs
 * `querySelectorAll` over every message *and* every member row, the attachment pass over every
 * media element and file card. So a single message cost several full document walks, on the main
 * thread of the client Shiver is trying to stay out of the way of.
 *
 * Batching through `requestAnimationFrame` collapses a burst of mutations into one pass, and
 * sharing the observer means the page is walked once per frame rather than once per registered
 * callback. The callbacks stay idempotent, which is what makes this safe: they are free to run when
 * nothing relevant changed, and the ones that write to the DOM write the same thing twice.
 *
 * Callers that need a first pass before anything mutates keep their own initial call — this only
 * replaces the observing.
 */
const domSettledCallbacks = new Set<() => void>();

let domSettledObserver: MutationObserver | null = null;
let domSettledScheduled = false;

function runDomSettled() {
  domSettledScheduled = false;

  for (const callback of domSettledCallbacks) {
    try {
      callback();
    } catch (error) {
      // one misbehaving pass must not stop the others: they are independent, and a page that has
      // moved on from what one of them expects is exactly when the rest still need to run
      console.error('[shiver] a dom-settled callback failed', error);
    }
  }
}

function onDomSettled(callback: () => void) {
  domSettledCallbacks.add(callback);

  if (domSettledObserver) return;

  domSettledObserver = new MutationObserver(() => {
    if (domSettledScheduled) return;

    domSettledScheduled = true;
    requestAnimationFrame(runDomSettled);
  });

  domSettledObserver.observe(document.body, { childList: true, subtree: true });
}

function defineHook<K extends keyof Window>(name: K, value: Window[K]) {
  try {
    // Cleared first. On desktop the bridge is an initialization script and there is never anything
    // here; on mobile it is evaluated after the page has loaded, so a page's own script may have
    // got here first and `defineProperty` over a non-configurable property would throw.
    delete window[name];

    Object.defineProperty(window, name, {
      value,
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch {
    // Either this ran twice — in which case the existing definition is ours and is the one to keep
    // — or a page locked the name before the bridge was evaluated, which only mobile's late
    // injection makes possible. The core treats what it reads back out of a page as a request
    // rather than a fact, and rations what it acts on, so the second case costs the hook rather
    // than the boundary.
  }
}

function install(shiver: ShiverConfig) {
  seedAutoLogin(shiver.token);

  if (shiver.role === 'dm') {
    installConversationView(shiver);

    return;
  }

  seedNotificationSettings();
  seedChannelRestore();

  // Both of these patch prototypes and touch no dom, so they run now rather than on ready — they
  // have to be in place before the page's own scripts are, which is the point of an
  // `initialization_script`. Anything that needs an element belongs in `whenDocumentReady` below.
  installSoundVolume(shiver.soundVolume);
  silenceMessagePing();

  let muted = new Set(shiver.muted);
  let state: SharkordState = {};
  const queue: QueuedNotification[] = [];
  const openQueue: string[] = [];
  const muteQueue: QueuedMute[] = [];
  /** when Shiver last saw a message in a channel, which is all it can know without the message list */
  const lastSeen = new Map<number, number>();
  let dms: DmChannel[] | null = null;
  let dmsSignature = '';

  const mutedNames = () =>
    new Set(
      (state.channels ?? [])
        .filter((channel) => muted.has(channel.id))
        .map((channel) => channel.name)
    );

  let syncedMutes: number[] | null = null;

  // a failed open must be reported, not swallowed: the page keeps showing whatever conversation was
  // there before, and Shiver would otherwise highlight a different one in its list
  let openDmFailure: string | null = null;

  // a failed open must be reported, not swallowed: the page keeps showing whatever conversation was
  // there before, and Shiver would otherwise highlight a different one in its list
  reportOpenDmFailure = (name) => {
    openDmFailure = name;
  };

  defineHook('__SHIVER_SET_MUTED__', (next) => {
    muted = new Set(next);
    paintMuted(mutedNames());

    // keep the server's copy in step with whatever Shiver just decided
    pushMutesToPlugin([...muted]);
  });

  defineHook('__SHIVER_SET_VOICE_LOCK__', (locked) => setVoiceLocked(locked));
  defineHook('__SHIVER_VOICE__', (action) => runVoiceAction(action));
  defineHook('__SHIVER_MARK_ALL_READ__', () => markAllChannelsRead(state));

  // the core takes everything queued since the last call and clears it, so nothing is delivered
  // twice and a page that is never drained cannot grow without bound
  defineHook('__SHIVER_DRAIN__', () => {
    const pendingDms = dms;
    const pendingSynced = syncedMutes;

    const pendingFailure = openDmFailure;

    dms = null;
    syncedMutes = null;
    openDmFailure = null;

    return {
      notifications: resolveDmChannels(queue, state).splice(0, queue.length),
      mutes: muteQueue.splice(0, muteQueue.length),
      dms: pendingDms,
      syncedMutes: pendingSynced,
      open: openQueue.splice(0, openQueue.length),
      openDmFailed: pendingFailure,
      ready: isClientReady(state),
      signedOut: isSignedOut(),
      fullscreen: document.fullscreenElement !== null,
      voice: readVoice(state),
      // An open conversation first: it is the one Sharkord does not report through
      // `selectedChannelId`, which goes on naming whatever ordinary channel was last chosen.
      viewingChannelId:
        openDmChannelId(state) ??
        (typeof state.selectedChannelId === 'number' ? state.selectedChannelId : null)
    };
  });

  defineHook('__SHIVER_OPEN_DM__', (name) => openDirectMessage(name));
  defineHook('__SHIVER_SELECT_CHANNEL__', (channelId) => selectChannelWhenReady(channelId));
  defineHook('__SHIVER_SET_READ_FLOOR__', (floor) => void storeReadFloor(floor));
  defineHook('__SHIVER_SET_DM_MODE__', (enabled) => setDmMode(enabled));
  defineHook('__SHIVER_SET_THEME__', (theme) => setTheme(theme));

  captureNotifications(queue, lastSeen, () => state, () => muted);

  whenDocumentReady(() => {
    if (shiver.theme) applyTheme(shiver.theme);

    reserveTopBarSpace();
    installAttachmentCards(shiver.minimiseAttachments);
    installAttachmentFocus();
    installVoiceColors();
    installVoiceLock(shiver.voiceLocked, () => state);
    installExternalLinks(openQueue);

    watchStore((next) => {
      state = next;

      paintMuted(mutedNames());

      const list = readDms(shiver.origin, next, lastSeen);
      // avatar urls carry an expiring access token and change on their own, so they are left out:
      // including them would republish the whole list every time one is reissued
      const signature = JSON.stringify(
        list.map(({ channelId, name, lastMessageAt }) => [channelId, name, lastMessageAt])
      );

      if (signature === dmsSignature) return;

      dmsSignature = signature;
      dms = list;
    });

    installRoleColors();

    watchChannelContextMenu(() => state, () => muted, muteQueue);

    installStatusButton();

    syncMutesWithPlugin([...muted]).then((merged) => {
      if (!merged) return;

      muted = new Set(merged);
      syncedMutes = merged;

      paintMuted(mutedNames());
    });

    // the channel list re-renders on scroll, selection and unread changes, each of which drops the
    // dimming, so it is reapplied whenever the sidebar's dom changes
    onDomSettled(() => paintMuted(mutedNames()));
  });
}

/**
 * The conversation view: a second page for the same server, showing one DM.
 *
 * It reports neither notifications nor DM lists. The server page for this entry already does both,
 * and two pages reporting the same server would double every notification and fight over the
 * inbox. All this page does is hide its own sidebar and open what Shiver asks for.
 */
function installConversationView(shiver: ShiverConfig) {
  const openQueue: string[] = [];

  installSoundVolume(shiver.soundVolume);
  silenceMessagePing();

  suppressNotifications();

  let openDmFailure: string | null = null;

  reportOpenDmFailure = (name) => {
    openDmFailure = name;
  };

  defineHook('__SHIVER_OPEN_DM__', (name) => openDirectMessage(name));
  defineHook('__SHIVER_SET_THEME__', (theme) => setTheme(theme));

  // Still drained, for two things. A conversation Shiver could not open is reported rather than
  // silently missed — and **which conversation is on screen**, which is the one fact only this page
  // has: the server page for this entry carries on reporting whatever channel it was left on.
  //
  // Everything else stays empty. The server page is already reporting it, and two pages answering
  // for one server would double every notification and fight over the inbox. `viewingChannelId` is
  // not one of those: it is not a notification, and the core reads it from this page alone — see
  // `mark_read_conversation_read` in `drain.rs`.
  defineHook('__SHIVER_DRAIN__', () => {
    const pendingFailure = openDmFailure;

    openDmFailure = null;

    return {
      notifications: [],
      mutes: [],
      dms: null,
      syncedMutes: null,
      open: openQueue.splice(0, openQueue.length),
      openDmFailed: pendingFailure,
      ready: false,
      signedOut: false,
      fullscreen: false,
      voice: null,
      // Read from the store rather than left null, which is what it used to be — so the core was
      // told nothing about the conversation being read, and reading a DM never cleared its
      // notification or moved the badge. The whole point of draining this page is here.
      viewingChannelId:
        typeof window.__SHARKORD_STORE__?.getState().selectedChannelId === 'number'
          ? (window.__SHARKORD_STORE__?.getState().selectedChannelId as number)
          : null
    };
  });

  whenDocumentReady(() => {
    if (shiver.theme) applyTheme(shiver.theme);

    reserveTopBarSpace();
    setDmMode(true);
    installRoleColors();

    // a conversation is as full of pictures as a channel is
    installAttachmentCards(shiver.minimiseAttachments);
    installAttachmentFocus();
    installVoiceColors();
    installExternalLinks(openQueue);

    // the opener waits for the client to connect on its own, so this can start immediately
    if (shiver.openDm) openDirectMessage(shiver.openDm);

    // the sidebar is re-rendered as the client connects, and DM mode has to survive that
    onDomSettled(() => setDmMode(true));
  });
}

/**
 * Sharkord auto-logs-in when these two keys are present, which is exactly the path Shiver wants: the
 * client connects with the session Shiver already obtained and the login screen never renders. If the
 * token is stale the client clears these itself and falls back to asking the user.
 */
function seedAutoLogin(token: string | null) {
  try {
    if (!token) {
      // Shiver holds no session for this server, so neither may the page. Without this a log out
      // would not take: the keys Shiver seeded on an earlier run outlive the webview, and the client
      // would sign the user straight back in with a token they just asked to be rid of.
      localStorage.removeItem('sharkord-auto-login');
      localStorage.removeItem('sharkord-auto-login-token');

      return;
    }

    localStorage.setItem('sharkord-auto-login', 'true');
    localStorage.setItem('sharkord-auto-login-token', token);
  } catch {
    // a page with storage blocked simply shows its own login screen
  }
}

/**
 * Turns Sharkord's own notifications on, **once**, and never again.
 *
 * Shiver's feed is built by intercepting the notifications Sharkord composes, and Sharkord only
 * composes them when its own settings say so — and those default to off. So without a first write
 * Shiver is silent out of the box and looks broken. That is the whole justification, and it justifies
 * exactly one write.
 *
 * It used to write on every page load, which meant the four switches in Sharkord's own settings
 * screen showed Shiver's preferences rather than the user's, and changing them did nothing that
 * survived a reload. A settings screen that quietly disagrees with itself is worse than one that
 * offers less. `seedChannelRestore` below has always done it this way; this is now the same shape.
 *
 * The consequence is deliberate: turn "all messages" off and Shiver's feed goes quiet for ordinary
 * messages, because it cannot intercept a notification that was never made. Someone who turned that
 * off asked for exactly that, and Shiver second-guessing them is how this went wrong in the first
 * place.
 *
 * Writes to this webview's storage only; the user's browser is untouched.
 */
function seedNotificationSettings() {
  const settings: Record<string, string> = {
    'sharkord-browser-notifications': 'true',
    'sharkord-browser-notifications-for-dms': 'true',
    'sharkord-browser-notifications-for-replies': 'true',
    // Shiver wants every channel message in the feed and filters with its own per-channel mutes,
    // so Sharkord must not narrow this to mentions first
    'sharkord-browser-notifications-for-mentions': 'false'
  };

  try {
    for (const [key, value] of Object.entries(settings)) {
      // absent means nobody has answered this yet. A value the user set — including one they set
      // back to what Shiver would have chosen — is an answer, and is left alone.
      if (localStorage.getItem(key) !== null) continue;

      localStorage.setItem(key, value);
    }
  } catch {
    // storage blocked, the page keeps its own defaults
  }
}

/**
 * Makes Sharkord reopen the channel the user was last in.
 *
 * Its own default is off (`features/app/slice.ts` reads the key with a `false` fallback), so a
 * freshly connected client sits on an empty server view. That breaks the message cache at exactly
 * the moment it is meant to help: Shiver shows the saved messages of a channel, the client finishes
 * connecting, and the channel it hands back is no channel at all.
 *
 * Only ever written when the key is *absent*. Seeding it unconditionally, the way the notification
 * settings are, would overrule a user who went into Sharkord's settings and deliberately turned it
 * off, and would do it again on every load.
 */
function seedChannelRestore() {
  try {
    if (localStorage.getItem('sharkord-auto-join-last-channel') === null) {
      localStorage.setItem('sharkord-auto-join-last-channel', 'true');
    }
  } catch {
    // storage blocked, the client keeps its own default
  }
}

/**
 * The tone Sharkord uses for an incoming message: `sfxMessageReceived` in `helpers/sounds.ts` is a
 * single sine at 600Hz, and nothing else it plays is a sine at that frequency except the first of
 * the three pulses in the screen-share chime.
 */
const MESSAGE_PING_TYPE: OscillatorType = 'sine';
const MESSAGE_PING_HZ = 600;

/**
 * Takes the incoming-message ping away from the page so Shiver can decide whether it sounds.
 *
 * Sharkord plays it on *every* message that is not the user's own, in a branch that never consults
 * a mute and never runs near the notification path (`features/server/messages/actions.ts`). So a
 * channel muted in Shiver would still ping, and the only place that decision can be made is here.
 * Shiver then plays its own ping for the notifications that survive the mute filter, which is also
 * what stops one message sounding twice.
 *
 * This is unconditional, and deliberately not tied to Shiver's "play notification sounds" setting.
 * Handing the ping back to the page when the user turns Shiver's sounds off would hand the mute back
 * with it, and turning notification sounds off would start making noise for muted channels.
 *
 * `createOscillator` is the seam: `helpers/sounds.ts` is the only thing in Sharkord that calls it,
 * while voice uses worklets, analysers and media stream sources, so nothing here can reach a call.
 * Only the message tone is swallowed — voice join and leave, the mic and deafen clicks, the webcam
 * and the disconnect chime all still sound, which the earlier blanket silencing took with it.
 *
 * The frequency is read from what Sharkord *asked* for rather than from `frequency.value`, which is
 * an audio-thread value and need not have caught up by the time `connect` is called.
 */
function silenceMessagePing() {
  const AudioContextCtor =
    window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;

  if (!AudioContextCtor?.prototype) return;

  const createOscillator = AudioContextCtor.prototype.createOscillator;
  // narrowed to the node-to-node overload: `connect` is also declared for AudioParam, and calling
  // it through `.call` otherwise resolves to that one and comes back as void
  const connect = AudioNode.prototype.connect as (
    this: AudioNode,
    destination: AudioNode
  ) => AudioNode;

  AudioContextCtor.prototype.createOscillator = function patched(this: AudioContext) {
    const oscillator = createOscillator.call(this);
    let requestedHz: number | null = null;

    const frequency = oscillator.frequency;
    const setValueAtTime = frequency.setValueAtTime.bind(frequency);

    frequency.setValueAtTime = (value: number, when: number) => {
      requestedHz = value;

      return setValueAtTime(value, when);
    };

    oscillator.connect = function patchedConnect(this: OscillatorNode, destination: AudioNode) {
      // swallowed rather than blocked: start() and stop() still work and make no sound, and the
      // destination is returned so the page's chained `.connect(...).connect(...)` keeps working
      if (this.type === MESSAGE_PING_TYPE && requestedHz === MESSAGE_PING_HZ) {
        return destination;
      }

      return connect.call(this, destination);
    } as OscillatorNode['connect'];

    return oscillator;
  };
}

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
 * Installed **before** `silenceMessagePing`: that one captures `AudioNode.prototype.connect` when
 * it runs, so it has to capture the patched version. Installed the other way round, the message
 * ping it lets through would connect straight to the speakers and ignore the setting.
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

  defineHook('__SHIVER_SET_SOUND_VOLUME__', (next: number) => {
    level = toLevel(next);

    for (const gain of masters) {
      gain.gain.value = level;
    }
  });
}

const TOP_BAR_RESERVE_ID = 'shiver-topbar-reserve';

/**
 * Shiver's notification bell floats over the right end of Sharkord's own top bar, so that bar is
 * given matching padding and its buttons shift left instead of being covered.
 *
 * The selector leans on Tailwind emitting literal class names for that bar. If a Sharkord update
 * changes them the padding stops applying and the bell overlaps the members toggle, which is
 * visible rather than silent.
 */
/**
 * Keeps Sharkord's top bar clear of Shiver's bell.
 *
 * The bell is its own 48x48 webview pinned to the window's top-right corner, over the page — so
 * without this it would sit on top of whatever Sharkord puts at the right end of its top bar, which
 * today is the members-sidebar toggle.
 *
 * **48px, matching `BELL_SIZE` in `webviews.rs` exactly**, and the two are one number in two places:
 * reserve less and the bell covers the toggle, reserve more and the toggle floats short of the edge
 * for no visible reason. The bell's own icon is 34px centred in its 48, so there is still 7px of
 * clear space beside the toggle without reserving any.
 *
 * **The `lg:grid` half is load-bearing, not decoration.** Two elements in the client carry both `h-12` and
 * `w-full`: this top bar, and the left sidebar's header — `flex w-full justify-between h-12`, whose
 * classes are in a different order, which is how a grep for the literal string missed it and why
 * `.h-12.w-full` alone quietly padded it too. The visible result was the server dropdown sitting
 * 48px in from the sidebar's edge rather than against it, for months, with no bell anywhere near it.
 * The grid class is the top bar's own and nothing else has it.
 *
 * It fails in the safe direction: if Sharkord restyles that bar and this stops matching, the bell
 * overlaps the members toggle, which is obvious and small. Matching too much is the failure that
 * hides.
 *
 * Written as an attribute selector rather than `.lg\\:grid` on purpose. A class selector has to
 * escape the colon, and a single backslash in a template literal is not an escape JavaScript keeps
 * — `\:` collapses to `:`, the rule becomes the pseudo-class `:grid`, and the whole thing is
 * dropped as invalid with nothing said. `[class~="lg:grid"]` matches the same class and has no
 * backslash to lose.
 */
/**
 * The second rule is the moderation view, and anything else Sharkord slides in from the right.
 *
 * `SheetContent` in `packages/ui/src/components/sheet.tsx` puts its close button at `top-4 right-4`
 * — a 16px cross, sixteen pixels from the corner, which is underneath Shiver's bell exactly. The bell
 * is its own webview floating over the page, so it does not merely overlap the cross: it takes the
 * click and the page never hears it. Escape still worked, because the moderation view binds that on
 * the document itself, which is precisely why the report was "you can't exit without pressing ESC".
 *
 * Padding cannot help here the way it does for the top bar. The sheet is `position: fixed` and
 * portalled to the body, so there is nothing containing it for Shiver to pad; the button is moved
 * instead, by the bell's width plus the gap it already had.
 *
 * Note there are no backticks in the css below. This is a template literal, and a backtick in a
 * comment inside one ends the string — which is what happened the first time this was written.
 */
function reserveTopBarSpace() {
  ensureStyle(TOP_BAR_RESERVE_ID).textContent = `
.h-12.w-full[class~="lg:grid"] { padding-right: 48px !important; }

/* the close button on a right-hand sheet, out from under Shiver's bell: 48px of bell plus its 16px gap */
[data-slot="sheet-content"][class~="right-0"] > button[class~="top-4"][class~="right-4"] {
  right: 64px !important;
}
`;
}

/**
 * Sharkord already decides what deserves a notification and calls `new Notification(...)` with the
 * author, channel and body composed. Wrapping the constructor gets Shiver that decision for free,
 * rather than Shiver second-guessing mentions, replies and DM rules.
 *
 * The native notification is suppressed: Shiver shows one unified feed instead of one popup per
 * server, and a muted channel produces neither.
 */
/**
 * Replaces `window.Notification` so a page's notifications go to Shiver instead of the desktop.
 *
 * Both kinds of page need this and for the same reason: whatever the page composes must not reach
 * the operating system. The server page turns it into a feed entry; the conversation view throws it
 * away. What neither may do is let it through.
 *
 * The wrapper claims permission because the client checks `Notification.permission === 'granted'`
 * before composing anything, and would otherwise compose nothing at all.
 */
function installNotificationWrapper(
  handle: (title: string, options?: NotificationOptions) => void
) {
  const Native = window.Notification;

  const Wrapped = function (title: string, options?: NotificationOptions) {
    handle(title, options);

    // the page still expects a Notification-shaped object back, and never sees this one shown
    return makeInertNotification();
  } as unknown as typeof Notification;

  Object.defineProperty(Wrapped, 'permission', { get: () => 'granted' });
  Wrapped.requestPermission = () => Promise.resolve('granted' as NotificationPermission);

  try {
    Object.defineProperty(window, 'Notification', {
      configurable: true,
      writable: true,
      value: Wrapped
    });
  } catch {
    // if the constructor cannot be replaced, leave the page's own notifications alone
    window.Notification = Native;
  }
}

/**
 * Turns the server page's notifications into Shiver's feed.
 *
 * Sharkord already decides what deserves one and composes the author, channel and body
 * (`features/server/messages/actions.ts`), so this inherits that decision rather than Shiver
 * re-deriving mention, reply and DM rules. The native popup is suppressed: Shiver shows one unified
 * feed instead of one popup per server, and a muted channel produces neither.
 */
function captureNotifications(
  queue: QueuedNotification[],
  lastSeen: Map<number, number>,
  getState: () => SharkordState,
  getMuted: () => Set<number>
) {
  installNotificationWrapper((title, options) => {
    const state = getState();
    const channelName = parseChannelName(title);
    const author = parseAuthor(title);
    const isDm = title.includes('(DM)');

    // the title is the only place the channel is named, so the id is recovered by matching it
    // against the store. `selectedChannelId` would be wrong here: a notification fires precisely
    // when the user is *not* looking at that channel.
    const channelId = isDm
      ? findDmChannelIdByUserName(state, author)
      : (state.channels ?? []).find(
          (channel) => !channel.isDm && channel.name === channelName
        )?.id ?? null;

    if (channelId !== null) {
      lastSeen.set(channelId, Date.now());
    }

    if (channelId !== null && getMuted().has(channelId)) return;

    queue.push({
      channelId,
      channelName,
      author,
      body: options?.body ?? '',
      iconUrl: options?.icon ?? null,
      isDm
    });
  });
}

/**
 * Throws away the conversation view's notifications.
 *
 * It is a second, complete Sharkord client for a server the server page is already reporting, so it
 * composes its own notification for every message that arrives there. Without a wrapper those went
 * straight to the operating system: a real desktop toast, with the system notification sound, for
 * every message including the ones Shiver had muted. Preloading the conversation views turned that
 * from something that happened only while the DM inbox was open into something permanent.
 *
 * Nothing is queued. The server page for this entry reports the feed, and two pages reporting one
 * server would double every notification in it.
 */
function suppressNotifications() {
  installNotificationWrapper(() => undefined);
}

/** Sharkord titles read `Author in #channel` or `Author (DM)`. */
function parseAuthor(title: string) {
  return (title.split(/ in #| \(DM\)/)[0] ?? title).trim();
}

function parseChannelName(title: string) {
  const match = title.match(/ in #(.+)$/);

  return match?.[1] ?? null;
}

/**
 * The page still expects a Notification-shaped object back. This one does nothing and is never
 * shown, so a suppressed notification cannot throw inside Sharkord.
 */
function makeInertNotification() {
  return {
    close: () => undefined,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    dispatchEvent: () => false
  } as unknown as Notification;
}

/**
 * Sharkord names a DM channel `DM - <userA>:<userB>` (`routers/dms/open-direct-message.ts`), which
 * is the only place the participants are recorded in anything the plugin store exposes. Everything
 * Shiver shows about a conversation is recovered from those two ids.
 */
function dmPartnerId(channel: SharkordChannel, ownUserId: number | undefined) {
  const match = channel.name.match(/^DM - (\d+):(\d+)$/);

  if (!match) return null;

  const [a, b] = [Number(match[1]), Number(match[2])];

  if (ownUserId === undefined) return a;

  return a === ownUserId ? b : a;
}

/**
 * Fills in the channel of any queued direct message whose channel could not be resolved yet.
 *
 * A notification is captured the moment Sharkord raises it, and for a direct message the channel is
 * recovered by looking the author up in the store — which, in the first seconds after a page
 * connects, has neither the user list nor the dm channels in it yet. So the channel came back null,
 * and a notification with no channel is one the core will **never** clear: reading a channel clears
 * notifications *for that channel*, and this one claimed to be for none.
 *
 * That is a badge that cannot be got rid of, and it was reported as exactly that. Retrying here
 * costs nothing — the queue is walked on the way out anyway — and by drain time the store is
 * usually populated.
 */
function resolveDmChannels(queue: QueuedNotification[], state: SharkordState) {
  for (const queued of queue) {
    if (!queued.isDm || queued.channelId !== null) continue;

    queued.channelId = findDmChannelIdByUserName(state, queued.author);
  }

  return queue;
}

/**
 * The conversation open in Sharkord's direct-message view, if one is.
 *
 * **`selectedChannelId` is not this.** Direct messages are tracked in a different slice entirely —
 * `app.selectedDmChannelId`, set by `setSelectedDmChannelId` — and none of that reaches the plugin
 * store, which only carries the *server* slice. Measured on a real client: `selectedChannelId` sat
 * at 26 for an entire session while a conversation on channel 29 was open and being read. Six
 * builds went out inferring "what is being read" from a value that does not follow a conversation.
 *
 * So it is read off the page instead. Sharkord marks the open row with `bg-accent`, and the row
 * carries the person's name — which is the same thing a direct message's notification is resolved
 * by, so the two agree by construction rather than by luck.
 *
 * The longest matching name wins, so "Test User" cannot be mistaken for "Test User 2".
 */
function openDmChannelId(state: SharkordState) {
  const rows = document.querySelectorAll<HTMLElement>('[data-testid="dm-item"]');

  for (const row of rows) {
    // `hover:bg-accent` is a different class, so this does not match a row merely under the pointer
    if (!row.classList.contains('bg-accent')) continue;

    const text = row.textContent ?? '';
    let best: string | null = null;

    for (const user of state.users ?? []) {
      if (!user.name || !text.includes(user.name)) continue;
      if (best === null || user.name.length > best.length) best = user.name;
    }

    if (best !== null) return findDmChannelIdByUserName(state, best);
  }

  return null;
}

function findDmChannelIdByUserName(state: SharkordState, name: string) {
  const user = (state.users ?? []).find((candidate) => candidate.name === name);

  if (!user) return null;

  const channel = (state.channels ?? []).find(
    (candidate) => candidate.isDm && dmPartnerId(candidate, state.ownUserId) === user.id
  );

  return channel?.id ?? null;
}

function fileUrl(origin: string, file: SharkordFile | null | undefined) {
  if (!file) return null;

  const base = `${origin}/public/${file.name}`;

  if (!file._accessToken) return encodeURI(base);

  const expires = file._accessTokenExpiresAt ? `&expires=${file._accessTokenExpiresAt}` : '';

  return encodeURI(`${base}?accessToken=${file._accessToken}${expires}`);
}

function readDms(
  origin: string,
  state: SharkordState,
  lastSeen: Map<number, number>
): DmChannel[] {
  const users = new Map((state.users ?? []).map((user) => [user.id, user]));

  return (state.channels ?? [])
    .filter((channel) => channel.isDm)
    .map((channel) => {
      const partner = users.get(dmPartnerId(channel, state.ownUserId) ?? -1);

      if (!partner) return null;

      return {
        channelId: channel.id,
        name: partner.name,
        iconUrl: fileUrl(origin, partner.avatar),
        lastMessageAt: lastSeen.get(channel.id) ?? null
      };
    })
    // a conversation whose partner is not in the store yet is skipped rather than published under
    // its raw `DM - 1:2` channel name: that name would show in the inbox and, worse, would never
    // match a row when Shiver tried to open it
    .filter((dm): dm is DmChannel => dm !== null)
    // the store's channel order is not stable, and an inbox that reorders under the pointer means
    // clicking one conversation and landing in another
    .sort(
      (a, b) => (b.lastMessageAt ?? 0) - (a.lastMessageAt ?? 0) || a.name.localeCompare(b.name)
    );
}

const DM_MODE_STYLE_ID = 'shiver-dm-mode';

/**
 * Hides Sharkord's own left sidebar while Shiver is showing its cross-server DM list beside this
 * page, so there is one list on screen instead of two. The conversation itself is untouched: the
 * server's client still renders it, Shiver has only narrowed the webview and hidden one panel.
 *
 * Keyed on `data-testid`, which is a stable contract in Sharkord rather than a styling detail.
 */
function setDmMode(enabled: boolean) {
  ensureStyle(DM_MODE_STYLE_ID).textContent = enabled
    ? '[data-testid="left-sidebar"] { display: none !important; }'
    : '';
}

/**
 * Whether a Sharkord DM row is the conversation with `name`.
 *
 * The name lives in the row's `truncate flex-1` span. It is not simply the row's first span:
 * `UserAvatar` renders a radix `AvatarFallback`, which is also a span and holds the user's
 * *initials* whenever their avatar image has not loaded. Matching that span compared a username
 * against initials, so every avatar-less user silently failed to open.
 */
function rowMatchesName(row: HTMLElement, name: string) {
  const named = row.querySelector<HTMLElement>('span.flex-1, span.truncate');

  if (named) return named.textContent?.trim() === name;

  // markup changed: fall back to any span that is exactly the name, never a substring
  return [...row.querySelectorAll('span')].some((span) => span.textContent?.trim() === name);
}

let openDmTimer: number | null = null;
/** set by install(), so the opener can hand a failure to the next drain */
let reportOpenDmFailure: (name: string) => void = () => undefined;

/**
 * Opens one conversation by driving Sharkord's own controls: the DM list is a view of app state
 * Shiver cannot set directly, but the controls that set it are right there.
 *
 * Sharkord mounts its DM list only while DM mode is on (`{dmsOpen ? <DirectMessages/> : ...}`), and
 * the only control for that is a *toggle*. So the toggle is pressed at most once, and only while no
 * rows exist at all. Pressing it because a particular row was missing is what made Shiver flip DM
 * mode on and off on every click.
 */
function openDirectMessage(name: string) {
  // a second request supersedes the first, rather than two loops racing to click different rows
  if (openDmTimer !== null) {
    window.clearInterval(openDmTimer);
    openDmTimer = null;
  }

  let toggled = false;
  let done = false;
  // a deadline, not an attempt count: on a freshly created webview this runs before the client has
  // connected, so it has to outlast sign-in, the websocket handshake and the first DM fetch
  const deadline = Date.now() + 25_000;

  const stop = () => {
    done = true;

    if (openDmTimer === null) return;

    window.clearInterval(openDmTimer);
    openDmTimer = null;
  };

  const attempt = () => {
    const rows = [...document.querySelectorAll<HTMLElement>('[data-testid="dm-item"]')];

    if (rows.length === 0) {
      // DM mode is off, or its list has not arrived yet. one press, never a second.
      if (!toggled) {
        const toggle = document.querySelector<HTMLElement>('[data-testid="dm-toggle"]');

        // only counts as pressed if the button was actually there. on a cold page it is not
        // rendered until the client connects, and marking it pressed regardless meant DM mode
        // never got turned on at all.
        if (toggle) {
          toggle.click();
          toggled = true;
        }
      }
    } else {
      const row = rows.find((candidate) => rowMatchesName(candidate, name));

      if (row) {
        row.click();
        stop();

        return;
      }
    }

    // DM mode is left open on whatever it was showing, which beats closing it, but Shiver is told so
    // it does not claim to have opened something it has not.
    if (Date.now() > deadline) {
      reportOpenDmFailure(name);
      stop();
    }
  };

  attempt();

  // only keep polling if the first try did not already land it
  if (!done) {
    openDmTimer = window.setInterval(attempt, 150);
  }
}

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
 * The bridge cannot call `executePluginAction` directly, because Sharkord reads the plugin id off
 * the calling stack frame and this script is not served from /plugin-bundle. So the request is
 * posted into the page and the plugin's own bundle makes the call.
 */
function callPlugin(action: string, payload?: unknown): Promise<PluginResponse['result'] | null> {
  if (!window.__SHIVER_PLUGIN__) return Promise.resolve(null);

  return new Promise((resolve) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;

    const done = (value: PluginResponse['result'] | null) => {
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
    window.postMessage(
      { source: PLUGIN_REQUEST, id, action, payload },
      window.location.origin
    );
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
 * The two are unioned rather than one overwriting the other: a mute made on this machine before the
 * plugin existed, and a mute made on another device, are both things the user asked for, and
 * silently dropping either would be the wrong call. After this the server's copy is the truth.
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

function pushMutesToPlugin(mutedChannels: number[]) {
  callPlugin('setMutedChannels', { mutedChannels }).catch(() => undefined);
}

/**
 * Whether the page has something for the user, so Shiver can stop covering it.
 *
 * Normally that means the client connected: `ownUserId` is only set once the session has
 * authenticated, and the server view only mounts once the client has routed to it, so a
 * half-connected client does not count.
 *
 * A visible login form counts too, and must. Shiver seeds a session so that form never appears, but
 * a token the server rejects sends the client straight back to it — and covering *that* with a
 * connecting view is a trap: the wait never ends, because it is waiting for a sign-in the user
 * cannot reach. Whatever the page is asking for, it has to be the thing on screen.
 */
/**
 * Whether the client has fallen back to asking for credentials.
 *
 * The login form alone is not enough to say so: it is what the page shows *while* auto-login is
 * still being attempted. The flag is what settles it — Shiver seeds `sharkord-auto-login` as `true`,
 * and the client sets it to `false` itself once it has given up on the token. So a form plus a flag
 * that is no longer `true` means the seeded session was refused, which is Shiver's cue to get another
 * one rather than let the user meet a login screen it promised they would not.
 *
 * With nothing seeded at all the flag is absent, which also reads as signed out. That is correct:
 * the core is the side that knows whether it holds credentials to do anything about it.
 */
function isSignedOut() {
  if (document.querySelector('[data-testid="connect-form"]') === null) return false;

  try {
    return localStorage.getItem('sharkord-auto-login') !== 'true';
  } catch {
    // storage blocked: the form is up and Shiver cannot tell why, so let the core try once
    return true;
  }
}

function isClientReady(state: SharkordState) {
  if (document.querySelector('[data-testid="connect-form"]') !== null) return true;

  if (typeof state.ownUserId !== 'number') return false;

  return document.querySelector('[data-testid="server-view"]') !== null;
}

/**
 * Matches a lucide icon by name.
 *
 * lucide-react puts `lucide-<kebab-name>` on every icon it renders, and newer versions add
 * `lucide-<kebab-name>-icon` alongside it. Both are accepted, and the class is compared as a whole
 * token rather than as a substring: `lucide-mic` is a prefix of `lucide-mic-off`, so a substring
 * test would read a muted microphone as an unmuted one.
 */
function hasLucideIcon(element: Element, name: string) {
  for (const icon of element.querySelectorAll('svg')) {
    if (icon.classList.contains(`lucide-${name}`)) return true;
    if (icon.classList.contains(`lucide-${name}-icon`)) return true;
  }

  return false;
}

type VoiceButtons = {
  mic: HTMLButtonElement | null;
  sound: HTMLButtonElement | null;
  leave: HTMLButtonElement | null;
};

/**
 * Finds Sharkord's own voice controls, which is how Shiver drives them.
 *
 * There is no store action for muting: `ownVoiceState` and its toggles live in the client's redux
 * store and its React context, neither of which is exposed. The buttons are, so Shiver clicks them.
 *
 * Mic and deafen are found as a *pair sharing a parent*, not simply as the first mic icon in the
 * sidebar. A voice channel's user list renders the same `MicOff` and `HeadphoneOff` icons once per
 * participant to show who is muted, so anything looser reads another user's state as the user's own
 * and mutes the wrong thing. Those indicators are plain elements; only the real controls are
 * buttons, and only the real controls sit side by side in one container.
 */
function findVoiceButtons(): VoiceButtons {
  const scope = document.querySelector('[data-testid="left-sidebar"]') ?? document.body;
  const buttons = [...scope.querySelectorAll<HTMLButtonElement>('button')];

  const isMic = (button: Element) => hasLucideIcon(button, 'mic') || hasLucideIcon(button, 'mic-off');
  const isSound = (button: Element) =>
    hasLucideIcon(button, 'headphones') || hasLucideIcon(button, 'headphone-off');

  const mic =
    buttons.find(
      (button) =>
        isMic(button) &&
        [...(button.parentElement?.children ?? [])].some(
          (sibling) => sibling !== button && isSound(sibling)
        )
    ) ?? null;

  const sound = mic
    ? ([...(mic.parentElement?.children ?? [])].find(
        (sibling) => sibling !== mic && isSound(sibling)
      ) as HTMLButtonElement | undefined) ?? null
    : null;

  // only rendered while a call is up, so it needs no pairing to be unambiguous
  const leave = buttons.find((button) => hasLucideIcon(button, 'phone-off')) ?? null;

  return { mic, sound, leave };
}

/**
 * The voice session on this server.
 *
 * The channel is authoritative: it comes from the plugin store. The mute flags are read off which
 * icon each control is currently showing, because Sharkord publishes the channel to plugins but not
 * `ownVoiceState`.
 */
function readVoice(state: SharkordState): VoiceSnapshot | null {
  const channelId = state.currentVoiceChannelId;

  if (typeof channelId !== 'number') return null;

  const { mic, sound } = findVoiceButtons();
  const channel = (state.channels ?? []).find((candidate) => candidate.id === channelId);

  return {
    channelId,
    channelName: channel?.name ?? null,
    // absent controls read as unmuted rather than as muted: claiming a live mic is muted would be
    // the more misleading of the two
    micMuted: mic ? hasLucideIcon(mic, 'mic-off') : false,
    soundMuted: sound ? hasLucideIcon(sound, 'headphone-off') : false,
    micLocked: mic?.disabled ?? false
  };
}

/** Clicks one of Sharkord's own voice controls on Shiver's behalf. */
function runVoiceAction(action: 'mic' | 'sound' | 'leave') {
  const buttons = findVoiceButtons();
  // matched exhaustively rather than falling through to `leave`: an action this bridge does not
  // recognise must do nothing, not hang up a call
  const button =
    action === 'mic'
      ? buttons.mic
      : action === 'sound'
        ? buttons.sound
        : action === 'leave'
          ? buttons.leave
          : null;

  // a disabled control is disabled for a reason Sharkord owns (no permission to speak, or the user
  // is deafened), and clicking it anyway would do nothing except desynchronise Shiver's own state
  if (!button || button.disabled) return;

  button.click();
}

/**
 * Marks every text channel on this server read.
 *
 * Sharkord marks a channel read as a side effect of selecting it (`setSelectedChannelId` calls
 * `markChannelAsRead` once the channel is the selected one), and `selectChannel` is part of the
 * plugin store's own published actions. So this is Shiver asking Sharkord to do the thing it already
 * does, not Shiver reaching into its state or speaking its protocol — which matters, because the
 * `channels.markAsRead` mutation is only reachable over the tRPC websocket.
 *
 * The user's channel is restored at the end. The whole loop is synchronous, so React batches it
 * into a single render and only the channel they were already on ever mounts: the view does not
 * visibly walk through the server.
 *
 * Voice channels and DMs are skipped. Sharkord only counts a channel read while it is *visible*,
 * and neither of those is made visible by selecting it.
 */
function markAllChannelsRead(state: SharkordState) {
  const selectChannel = window.__SHARKORD_STORE__?.actions?.selectChannel;

  if (!selectChannel) return;

  const previous = state.selectedChannelId;
  const channels = (state.channels ?? []).filter(
    (channel) => !channel.isDm && channel.type === 'TEXT'
  );

  for (const channel of channels) {
    // a channel with nothing unread returns early inside Sharkord, so this costs it no request
    selectChannel(channel.id);
  }

  if (typeof previous === 'number') selectChannel(previous);
}

/**
 * Puts the page on one channel, for a notification the user clicked in Shiver's feed.
 *
 * `selectChannel` is Sharkord's own published action — the same one `markAllChannelsRead` uses — so
 * this is asking the client to do what clicking the channel does, including marking it read, rather
 * than Shiver reaching into its state.
 *
 * Retried on a deadline rather than attempted once, for the same reason opening a DM is: the page
 * may have been built moments ago by the very click being handled, and the store's actions do not
 * exist until the client has connected. A channel the server does not have is not an error worth
 * reporting — it is a notification about a channel since deleted, and the retry simply runs out.
 */
function selectChannelWhenReady(channelId: number) {
  const deadline = Date.now() + 25_000;

  const attempt = () => {
    // the store is read here rather than through a passed-in accessor: this is called from the
    // core at an arbitrary moment, not from inside something that already holds one
    const store = window.__SHARKORD_STORE__;
    const selectChannel = store?.actions?.selectChannel;
    const known = (store?.getState().channels ?? []).some(
      (channel: SharkordChannel) => channel.id === channelId
    );

    if (selectChannel && known) {
      selectChannel(channelId);

      return true;
    }

    return Date.now() > deadline;
  };

  if (attempt()) return;

  const timer = window.setInterval(() => {
    if (attempt()) window.clearInterval(timer);
  }, 250);
}

/**
 * Stores this user's unread floor for this server, where the companion plugin can hold it.
 *
 * **This is the write half of the shared badge.** The core reads the floor back on every connection
 * (`plugins.getUserData`, a query, which is what lets its socket stay read-only) and measures the
 * badge from it. Writing is a mutation, so it happens here instead: a page is what the user opened,
 * and opening a server is the only thing that moves the floor.
 *
 * `getUserData`/`setUserData` are called directly rather than through the plugin relay, because
 * they take the plugin id as an argument. `executePluginAction` is the one that reads it off the
 * calling stack frame and therefore cannot be called from this script.
 *
 * The row is read and spread before writing, because a write replaces the whole of it: the muted
 * channel list lives in the same row and must survive this.
 *
 * Silent on failure, and there are two ordinary ones: no plugin installed, and a plugin installed
 * but switched off — `setUserData` answers NOT_FOUND for both. Neither is worth telling anybody
 * about, and each device still has its own floor to fall back on.
 */
async function storeReadFloor(floor: Record<string, number>) {
  const actions = window.__SHARKORD_STORE__?.actions;

  if (!actions?.getUserData || !actions?.setUserData) return;

  try {
    const stored = (await actions.getUserData(SHIVER_PLUGIN_ID)) ?? {};

    await actions.setUserData(SHIVER_PLUGIN_ID, { ...stored, readFloor: floor });
  } catch {
    // no plugin here, or it is switched off. the local floor still applies.
  }
}

/** The id Shiver's companion plugin installs under. Matches `SHIVER_PLUGIN_ID` in the rust core. */
const SHIVER_PLUGIN_ID = 'shiver';

const VOICE_NOTICE_ID = 'shiver-voice-notice';
let voiceLocked = false;
let voiceNoticeTimer: number | null = null;

/** Called by the core when the user joins or leaves a call on some other server. */
function setVoiceLocked(locked: boolean) {
  voiceLocked = locked;
}

/**
 * Refuses a voice join while another server holds the call.
 *
 * A user in a call on server A cannot join voice on server B. Undoing it afterwards was
 * the alternative, and it would mean genuinely joining first: two live mediasoup sessions, both
 * servers seeing the user arrive, and one of them then seeing them vanish. Refusing the click is
 * the only version where the second join never happens.
 *
 * It runs in the capture phase on `document`, which is above the root React attaches its own
 * handlers to, so stopping the event there means Sharkord's channel-select never runs. Voice rows
 * are told apart from text rows through the store rather than the markup, because a channel row
 * carries its name but not its id or its type.
 */
function installVoiceLock(initial: boolean, getState: () => SharkordState) {
  voiceLocked = initial;

  document.addEventListener(
    'click',
    (event) => {
      if (!voiceLocked) return;

      const row = (event.target as Element | null)?.closest?.('[data-testid="channel-item"]');

      if (!row) return;

      const name = row.textContent?.trim() ?? '';
      const isVoiceRow = (getState().channels ?? []).some(
        (channel) => channel.type === 'VOICE' && channel.name === name
      );

      if (!isVoiceRow) return;

      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation();

      showVoiceNotice();
    },
    true
  );
}

/**
 * Says why a voice channel did not open.
 *
 * Shiver's own chrome cannot be used here: the rail is in another webview and this click happened
 * inside the server's page, so the message has to appear where the user clicked. It deliberately
 * does not name the server holding the call, which this page is never told.
 */
function showVoiceNotice() {
  const existing = document.getElementById(VOICE_NOTICE_ID);
  const notice = existing ?? document.createElement('div');

  notice.id = VOICE_NOTICE_ID;
  notice.textContent = 'You are already in a voice channel on another server in Shiver.';
  notice.setAttribute('role', 'status');
  notice.setAttribute(
    'style',
    [
      'position: fixed',
      'left: 50%',
      'bottom: 24px',
      'transform: translateX(-50%)',
      'z-index: 2147483647',
      'padding: 10px 16px',
      'border-radius: 8px',
      'font-size: 13px',
      'pointer-events: none',
      'background: var(--popover, #171717)',
      'color: var(--popover-foreground, #fafafa)',
      'border: 1px solid var(--border, rgba(250,250,250,0.12))',
      'box-shadow: 0 8px 24px rgba(0,0,0,0.35)'
    ].join(';')
  );

  if (!existing) document.body.appendChild(notice);

  if (voiceNoticeTimer !== null) window.clearTimeout(voiceNoticeTimer);

  voiceNoticeTimer = window.setTimeout(() => {
    notice.remove();
    voiceNoticeTimer = null;
  }, 4000);
}

const MUTE_CLASS = 'shiver-muted-channel';
const MUTE_STYLE_ID = 'shiver-mute-style';

/**
 * Darkens muted channels in Sharkord's own channel list.
 *
 * Sharkord's channel rows carry `data-testid="channel-item"` but no channel id, so the row is
 * matched on the visible channel name. That is the weak point: two channels with the same name in
 * different categories dim together. Muting itself is keyed on the channel id and stays exact, so
 * this only ever affects the dimming.
 */
function paintMuted(mutedNames: Set<string>) {
  // Sharkord's own menu items highlight through `focus:bg-accent`, which relies on radix moving
  // real dom focus between the items it manages. Shiver's injected item is not one of those and never
  // receives that focus, so it gets an equivalent hover rule using the page's own theme variables.
  ensureStyle(MUTE_STYLE_ID).textContent = `
.${MUTE_CLASS} { opacity: 0.45; }
/* Sharkord's own unread pill. Muting a channel in Shiver stops its notification and its sound, but
   the server still counts the message and the channel list still marks it, which reads as the mute
   not having worked. The count itself is the server's and is left alone — this only stops a muted
   channel announcing itself. */
.${MUTE_CLASS} [data-testid="unread-count"] { display: none !important; }
.${SHIVER_MENU_ITEM}:hover, .${SHIVER_MENU_ITEM}:focus {
  background-color: var(--accent);
  color: var(--accent-foreground);
}
/* The reaction you are part of. Sharkord says so with a one-pixel border and nothing else, which is
   easy to miss at a glance in a row of five; this tints the whole pill in the accent instead, so it
   reads as yours without having to be read at all. Only a colour: the size and spacing are
   Sharkord's, and a pill that changed shape would move the ones beside it. */
${REACTED_PILL} {
  background-color: color-mix(in srgb, var(--primary) 22%, transparent) !important;
  border-color: var(--primary) !important;
}
`;

  const rows = document.querySelectorAll<HTMLElement>('[data-testid="channel-item"]');

  for (const row of rows) {
    const name = row.textContent?.trim() ?? '';

    row.classList.toggle(MUTE_CLASS, mutedNames.has(name));
  }
}

const SHIVER_MENU_ITEM = 'shiver-menu-item';

/**
 * Adds "Mute in Shiver" to Sharkord's own channel context menu.
 *
 * Sharkord's menu is a Radix portal, so Shiver waits for one to appear right after a right click on a
 * channel row and appends one item styled like its siblings. Nothing existing is replaced or
 * removed, so if this stops matching a future Sharkord the menu is simply the stock one again.
 */
/**
 * How long Shiver waits for Sharkord's own channel menu before drawing its own.
 *
 * Sharkord only wraps a channel row in a context menu for someone who can manage channels
 * (`components/context-menus/channel` hands its children back untouched otherwise), so a member
 * with no permissions got no menu at all — and muting, which is Shiver's and needs no permission,
 * was unreachable for exactly the people most likely to want it.
 */
const SHARKORD_MENU_GRACE_MS = 250;
/**
 * How long after a right-click a `[role="menu"]` is still taken to belong to it.
 *
 * Longer than the grace on purpose: past the grace Shiver has already drawn its own menu, and this is
 * the window in which a slower Sharkord menu can still arrive, take the mute item, and send Shiver's
 * away again.
 */
const SHARKORD_MENU_WINDOW_MS = 2000;

function watchChannelContextMenu(
  getState: () => SharkordState,
  getMuted: () => Set<number>,
  muteQueue: QueuedMute[]
) {
  let pendingChannel: SharkordChannel | null = null;
  let fallback = 0;

  /**
   * The channel the last right-click was on, and when — kept past `settle`.
   *
   * `pendingChannel` answers "is Shiver still waiting to hear whether Sharkord has a menu", and it is
   * cleared the moment Shiver stops waiting. That is the wrong thing to test when Sharkord's menu
   * turns up *late*: the answer is no, and the item quietly did not get added while Shiver's own menu
   * sat there beside it. This one answers "what was clicked", which stays true either way.
   */
  let lastChannel: SharkordChannel | null = null;
  let lastAt = 0;

  const settle = () => {
    window.clearTimeout(fallback);

    fallback = 0;
    pendingChannel = null;
  };

  // The event this listener decided to cancel, cancelled further down by the bubble listener.
  let cancelNativeFor: Event | null = null;

  // Capture, and it has to be: what happens here is bookkeeping, and it has to happen *before*
  // Sharkord's menu appears. Radix renders its menu during React's dispatch, and a MutationObserver
  // callback runs at the microtask checkpoint after the listener that caused it — so a `pending`
  // set any later than this is set after the observer has already looked and found nothing.
  document.addEventListener(
    'contextmenu',
    (event) => {
      const row = (event.target as Element | null)?.closest?.('[data-testid="channel-item"]');

      settle();
      closeOwnMenu();

      // whatever the last press marked is over; holding it would keep that event alive for nothing
      cancelNativeFor = null;

      if (!row) return;

      const name = row.textContent?.trim() ?? '';

      pendingChannel =
        (getState().channels ?? []).find(
          (channel) => !channel.isDm && channel.name === name
        ) ?? null;

      if (!pendingChannel) return;

      lastChannel = pendingChannel;
      lastAt = Date.now();

      // Marked for cancelling, but not cancelled here — see the bubble listener below.
      cancelNativeFor = event;

      const { clientX, clientY } = event;

      // Sharkord's menu, where the user is allowed one, lands within a frame or two. Shiver's own is
      // what happens when none does.
      fallback = window.setTimeout(() => {
        const channel = pendingChannel;

        settle();

        if (!channel) return;

        // Sharkord's menu may already be open from a right-click a moment ago on the same row.
        // Radix moves the menu it has rather than making another, so nothing is added to the
        // document and the observer above never hears about it — and Shiver drew a second menu
        // beside a perfectly good one. If one is on screen, it is the menu for this click.
        const open = openMenuOnScreen();

        if (open) {
          addMuteItem(open, channel, getMuted(), muteQueue);

          return;
        }

        openOwnMenu(channel, clientX, clientY, getMuted(), muteQueue);
      }, SHARKORD_MENU_GRACE_MS);
    },
    true
  );

  /**
   * Kills the webview's own menu over a channel row, once everyone else has had the event.
   *
   * Sharkord cancels the native menu itself wherever it has a menu of its own, so a user who can
   * manage channels never saw it. A user who cannot got no Sharkord menu, nothing cancelled
   * anything, and WebView2 drew its "Back / Reload / Save as" menu on top of Shiver's — which meant
   * the people who most need muting were the only ones who could not reach it.
   *
   * **This is a separate listener on the bubble phase, and has to be.** Cancelling from the capture
   * listener above looked equivalent — the event still reaches React either way — but Sharkord's
   * menu is Radix, and Radix composes its handlers with `composeEventHandlers`, which skips its own
   * handler when the event is *already* `defaultPrevented`. Shiver cancelling first therefore meant
   * Radix never opened at all: an admin right-clicking a channel got no Edit and no Delete, only
   * Shiver's fallback menu with Mute in it. Cancelling after React's dispatch suppresses the native
   * menu just as well, because the default action is decided once the whole dispatch is over —
   * which is why the usual "disable right click" snippet is a plain document listener like this.
   */
  document.addEventListener('contextmenu', (event) => {
    if (event !== cancelNativeFor) return;

    cancelNativeFor = null;
    event.preventDefault();
  });

  new MutationObserver((records) => {
    // A menu belongs to the right-click that just happened, not to one from a minute ago. Radix's
    // own is on screen within a frame or two; the window is generous because being late is exactly
    // the case this exists for.
    const channel = Date.now() - lastAt < SHARKORD_MENU_WINDOW_MS ? lastChannel : null;

    if (!channel) return;

    for (const record of records) {
      for (const node of record.addedNodes) {
        if (!(node instanceof HTMLElement)) continue;

        const menu = node.matches('[role="menu"]')
          ? node
          : node.querySelector<HTMLElement>('[role="menu"]');

        if (!menu || menu.querySelector(`.${SHIVER_MENU_ITEM}`)) continue;

        addMuteItem(menu, channel, getMuted(), muteQueue);

        // Theirs arrived, so Shiver's own is not needed — and if it is already on screen, because
        // theirs took longer than the grace, it has to go. Two menus for one click, one of them a
        // subset of the other, is what this looked like before: Shiver's own beside Sharkord's, both
        // offering the mute. `closeOwnMenu` is safe when nothing is open.
        settle();
        closeOwnMenu();
      }
    }
  }).observe(document.body, { childList: true, subtree: true });
}

const OWN_MENU_ID = 'shiver-channel-menu';

function closeOwnMenu() {
  document.getElementById(OWN_MENU_ID)?.remove();
}

/**
 * Shiver's own channel menu, for the users Sharkord does not give one.
 *
 * Deliberately just the one item. This is not a reimplementation of Sharkord's menu — the things it
 * offers are things only a moderator may do, and Shiver has no business drawing them for someone who
 * may not. Muting is Shiver's own, on this device, and needs no permission from anybody.
 *
 * In a closed shadow root so the page cannot restyle it, and positioned in the corner nearest the
 * pointer so it never hangs off the window.
 */
function openOwnMenu(
  channel: SharkordChannel,
  x: number,
  y: number,
  muted: Set<number>,
  muteQueue: QueuedMute[]
) {
  closeOwnMenu();

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const style = document.createElement('style');
  const menu = document.createElement('div');
  const item = document.createElement('button');
  const isMuted = muted.has(channel.id);

  host.id = OWN_MENU_ID;

  style.textContent = `
:host { position: fixed; inset: 0; z-index: 2147483646; }
.menu { position: absolute; min-width: 180px; padding: 4px; border-radius: 8px;
  border: 1px solid rgb(255 255 255 / 12%); background: #1f1f1f; color: #fafafa;
  box-shadow: 0 12px 32px rgb(0 0 0 / 55%);
  font: 500 13px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
.item { display: block; width: 100%; padding: 8px 10px; border: none; border-radius: 6px;
  background: none; color: inherit; font: inherit; text-align: left; cursor: default; }
.item:hover { background: #333333; }
`;

  item.className = 'item';
  item.type = 'button';
  item.textContent = isMuted ? 'Unmute in Shiver' : 'Mute in Shiver';

  item.addEventListener('click', () => {
    muteQueue.push({ channelId: channel.id, muted: !isMuted });
    closeOwnMenu();
  });

  menu.className = 'menu';
  menu.style.left = `${Math.min(x, window.innerWidth - 200)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - 60)}px`;
  menu.append(item);
  root.append(style, menu);
  document.body.append(host);

  // the next click anywhere, a scroll, or escape puts it away — the same ways the page's own menus
  // are dismissed, so it never becomes the one thing left on screen
  const dismiss = () => {
    closeOwnMenu();

    document.removeEventListener('mousedown', dismiss, true);
    document.removeEventListener('scroll', dismiss, true);
    document.removeEventListener('keydown', onKey, true);
  };

  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') dismiss();
  };

  document.addEventListener('mousedown', dismiss, true);
  document.addEventListener('scroll', dismiss, true);
  document.addEventListener('keydown', onKey, true);
}

/**
 * Sharkord's own menu, if one is on screen.
 *
 * `data-state` is Radix's own word for it, and a menu it has closed can stay in the document —
 * measuring it as well means a leftover cannot be mistaken for an open one. Shiver's own menu is in a
 * closed shadow root and is invisible to this, which is what makes it safe to ask.
 */
function openMenuOnScreen(): HTMLElement | null {
  for (const menu of document.querySelectorAll<HTMLElement>('[role="menu"]')) {
    if (menu.dataset.state === 'closed') continue;

    const box = menu.getBoundingClientRect();

    if (box.width > 0 && box.height > 0) return menu;
  }

  return null;
}

function addMuteItem(
  menu: HTMLElement,
  channel: SharkordChannel,
  muted: Set<number>,
  muteQueue: QueuedMute[]
) {
  // Replaced rather than skipped when one is already there. Radix reuses the menu it has for the
  // next right-click, so an item left from the last one would name the wrong channel and mute it.
  menu.querySelector(`.${SHIVER_MENU_ITEM}`)?.remove();

  const sibling = menu.querySelector<HTMLElement>('[role="menuitem"]');
  const item = document.createElement('div');
  const isMuted = muted.has(channel.id);

  item.setAttribute('role', 'menuitem');
  // focusable so it also highlights when reached with the keyboard, the way radix's own items do
  item.tabIndex = -1;
  item.className = `${sibling?.className ?? ''} ${SHIVER_MENU_ITEM}`.trim();
  item.textContent = isMuted ? 'Unmute in Shiver' : 'Mute in Shiver';

  item.addEventListener('mouseenter', () => item.focus());

  item.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();

    muteQueue.push({ channelId: channel.id, muted: !isMuted });

    // let the page close its own menu the way it would for any other item
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  });

  menu.appendChild(item);
}

const THEME_STYLE_ID = 'shiver-theme';

/**
 * Repaints the server's own client in the user's colours.
 *
 * Sharkord renders its app under a hard-coded `dark` class, so Shiver's overrides have to target that
 * as well as `:root` and win on specificity rather than on order alone.
 *
 * It covers the whole surface set, not just the page background: sidebar, cards, popovers, inputs
 * and borders are all derived from the chosen colour with `color-mix`, so one picked colour themes
 * the client rather than leaving default greys sitting next to it. The accent drives the things
 * Sharkord treats as primary, and its foreground is chosen for contrast so a light accent does not
 * end up with white text on it.
 */
/**
 * Follows the links the webview will not.
 *
 * Sharkord opens files and outside links with `target="_blank"` — an attachment card, a link in a
 * message, an open-graph preview. That asks for a *new window*, which Shiver has nowhere to put and
 * which the webview refuses by default, so the click did nothing at all and did it silently.
 *
 * There is a native hook for this (`on_new_window`), and Shiver sets it; this exists because it is
 * the click itself that is caught, before the webview has to decide anything. The address is queued
 * and the core hands it to the browser on its next drain — the same place every other link out of
 * Sharkord goes, and the thing that knows how to save a file.
 *
 * Only `http(s)`, and only where the page meant to leave: an in-place link is still a navigation,
 * which the guard already handles.
 */
function installExternalLinks(queue: string[]) {
  document.addEventListener(
    'click',
    (event) => {
      if (!event.isTrusted || event.defaultPrevented) return;
      // a modified click is the user asking their own way; leave it to the page
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }

      const anchor = (event.target as Element | null)?.closest?.('a');

      if (!(anchor instanceof HTMLAnchorElement) || anchor.target !== '_blank') return;

      // A control *inside* the link is not the link.
      //
      // Sharkord's attachment card is an `<a target="_blank">` to the file with a delete button
      // sitting inside it, and that button's own handler cancels the anchor. It never got the
      // chance: this listener captures, so it ran first, called `stopPropagation`, and the click
      // never reached React at all — pressing delete opened the attachment in the browser instead
      // of deleting it. Anything a page put inside a link to be pressed in its own right is left
      // alone, and a plain click on the link is still Shiver's to hand over.
      const control = (event.target as Element | null)?.closest?.(
        'button, input, select, textarea, label, [role="button"], [contenteditable]'
      );

      if (control && control !== anchor && anchor.contains(control)) return;

      const href = anchor.href;

      if (!/^https?:/i.test(href)) return;

      event.preventDefault();
      event.stopPropagation();

      queue.push(href);
    },
    true
  );
}

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
 * `border-border` when you have reacted, `border-none` when you have not — which is easy to miss.
 */
const REACTED_PILL = '[class~="h-9"][class~="border-border"]';

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
 *   schema and a name is near enough white already.
 *
 * Names are matched by their text, because that is all the dom has: Sharkord puts no user id on a
 * message or a member row. Two people with the same display name would be coloured alike, which is
 * the same limitation the muted-channel dimming already lives with.
 */
function installRoleColors() {
  let colors = new Map<string, string>();

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
    // *you* were pinged, which is a different thing from who somebody is, and worth more than the
    // colour of your own role.
    for (const chip of document.querySelectorAll<HTMLElement>(MENTION_CHIP)) {
      const name = chip.textContent?.trim().replace(/^@/, '') ?? '';

      if (!name || name === ownName) continue;

      applyNamed(chip, name);
    }
  };

  let ownName = '';

  watchStore((state) => {
    const users = state.users ?? [];
    const roles = state.roles ?? [];
    const next = new Map<string, string>();

    for (const user of users) {
      const color = colorFor(user, roles);

      if (color) next.set(user.name, color);
    }

    // kept so a mention of yourself can be left as Sharkord painted it
    ownName = users.find((user) => user.id === state.ownUserId)?.name ?? '';
    colors = next;

    paint();
  });

  // Messages and members are drawn as they arrive and redrawn as the user scrolls, and neither
  // touches the store, so the dom is what has to be watched for the rest.
  onDomSettled(paint);
}

function applyTheme({ themeColor, accentColor, textColor }: ShiverTheme) {
  const mixInto = (color: string) => (isLightColor(color) ? '#000' : '#fff');
  const lift = (percent: number) =>
    `color-mix(in srgb, ${themeColor} ${percent}%, ${mixInto(themeColor)})`;

  // The user's own text colour where they have chosen one, and otherwise whichever of black or
  // white their background can carry — which is what this always did.
  const foreground = textColor ?? (isLightColor(themeColor) ? '#171717' : '#fafafa');
  const dim = `color-mix(in srgb, ${foreground} 65%, ${themeColor})`;

  const variables: Record<string, string> = {
    '--background': themeColor,
    '--foreground': foreground,
    '--sidebar': lift(90),
    '--sidebar-foreground': foreground,
    '--card': lift(90),
    '--card-foreground': foreground,
    '--popover': lift(88),
    '--popover-foreground': foreground,
    '--muted-foreground': dim,
    '--muted': lift(84),
    '--secondary': lift(84),
    '--accent': lift(80),
    '--accent-foreground': foreground,
    '--sidebar-accent': lift(80),
    '--input': lift(78),
    '--border': `color-mix(in srgb, ${foreground} 12%, transparent)`,
    '--sidebar-border': `color-mix(in srgb, ${foreground} 12%, transparent)`,
    '--primary': accentColor,
    '--primary-foreground': isLightColor(accentColor) ? '#171717' : '#fafafa',
    '--sidebar-primary': accentColor,
    '--ring': accentColor,
    '--sidebar-ring': accentColor
  };

  // Set through the CSSOM rather than built as a string. Interpolating these into a `<style>`
  // element's `textContent` meant a value containing `}` closed the rule block and everything
  // after it became arbitrary CSS in this page; `setProperty` has no such seam — a malformed value
  // is rejected by the parser and the property keeps what it had.
  for (const rule of themeRules()) {
    for (const [name, value] of Object.entries(variables)) {
      rule.style.setProperty(name, value, 'important');
    }
  }
}

/**
 * The two empty rules `applyTheme` writes into, created once.
 *
 * `:root` and `.dark`, because Sharkord renders its app under a hard-coded `dark` class and
 * Shiver's overrides have to win on specificity rather than on order alone.
 */
function themeRules(): CSSStyleRule[] {
  const style = ensureStyle(THEME_STYLE_ID) as HTMLStyleElement;
  const sheet = style.sheet;

  if (!sheet) return [];

  if (sheet.cssRules.length === 0) {
    sheet.insertRule(':root {}', 0);
    sheet.insertRule('.dark {}', 1);
  }

  return Array.from(sheet.cssRules).filter(
    (rule): rule is CSSStyleRule => rule instanceof CSSStyleRule
  );
}

/**
 * Relative luminance, matching Shiver's own `theme.ts` so both sides pick the same foreground.
 *
 * **Including the fallback**, which is the half that did not match: this returned `false` where
 * `theme.ts` returns `true`, so anything that is not exactly six hex digits — a `#fff` shorthand,
 * a value hand-edited into `servers.json` — gave Shiver's chrome the light-background foreground
 * and the server page the dark one. Opposite text colours in the two halves of one window, under a
 * comment asserting they agreed.
 */
function isLightColor(hex: string) {
  const value = hex.replace('#', '');

  if (value.length !== 6) return true;

  const channel = (offset: number) => {
    const srgb = parseInt(value.slice(offset, offset + 2), 16) / 255;

    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };

  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4) > 0.35;
}

/** Called by the core when the user changes colours, so pages retheme without reloading. */
function setTheme(theme: ShiverTheme | null) {
  if (theme) {
    applyTheme(theme);

    return;
  }

  // back to Sharkord's own colours: the rules stay, emptied, so nothing has to be rebuilt if
    // the user picks a theme again
    for (const rule of themeRules()) {
      while (rule.style.length > 0) rule.style.removeProperty(rule.style.item(0));
    }
}

/**
 * The plugin store is published by the client's own entry point, which may not have run yet when
 * an initialization script executes, so this waits for it rather than assuming it.
 */
function watchStore(onChange: (state: SharkordState) => void) {
  const start = () => {
    const store = window.__SHARKORD_STORE__;

    if (!store) return false;

    const read = () => {
      try {
        onChange(store.getState());
      } catch {
        // a store read during teardown is not worth reporting
      }
    };

    store.subscribe(read);
    read();

    return true;
  };

  if (start()) return;

  const timer = window.setInterval(() => {
    if (start()) window.clearInterval(timer);
  }, 500);
}

/* ─────────────────────────── the status button ─────────────────────────── */

const STATUS_BUTTON_ID = 'shiver-status-button';
const STATUS_POPOVER_ID = 'shiver-status-popover';
const STATUS_STYLE_ID = 'shiver-status-style';

/** Sharkord's own settings gear, which this button sits beside. A documented test id. */
const SETTINGS_TRIGGER = '[data-testid="user-settings-trigger"]';

/**
 * A way to set your status without crossing the app to find it.
 *
 * The status itself is the plugin's and is set in Sharkord's own user settings, which is the route
 * that works in any browser. This is the short one: a button beside the settings gear, where the
 * user already is when they think about themselves rather than about a channel.
 *
 * Shiver draws it, so it exists only inside Shiver — the settings-screen field remains the way anyone
 * else does it. It is the same trade as the mute item in the channel menu: Shiver adds a shortcut to
 * something the plugin owns rather than owning the thing.
 *
 * Nothing happens on a server without the plugin. `callPlugin` answers null there, so the button is
 * never added and no space is taken by a control that could not work.
 */
function installStatusButton() {
  ensureStyle(STATUS_STYLE_ID).textContent = `
#${STATUS_POPOVER_ID} {
  position: fixed; z-index: 2147483646; width: 280px; padding: 12px;
  border-radius: 10px; display: flex; flex-direction: column; gap: 8px;
  background: var(--popover, #1f1f1f); color: var(--popover-foreground, #fafafa);
  border: 1px solid var(--border, rgb(255 255 255 / 12%));
  box-shadow: 0 12px 32px rgb(0 0 0 / 45%);
  font: 500 13px/1.4 system-ui, -apple-system, "Segoe UI", sans-serif;
}
#${STATUS_POPOVER_ID} label { color: var(--muted-foreground, #a1a1a1); font-size: 12px; }
#${STATUS_POPOVER_ID} input {
  width: 100%; box-sizing: border-box; padding: 7px 9px; border-radius: 7px;
  background: var(--input, rgb(255 255 255 / 6%)); color: inherit; font: inherit;
  border: 1px solid var(--border, rgb(255 255 255 / 14%)); outline: none;
}
#${STATUS_POPOVER_ID} .shiver-status-note { margin: 0; color: #f87171; font-size: 12px; }
#${STATUS_POPOVER_ID} .shiver-status-row { display: flex; gap: 8px; justify-content: flex-end; }
#${STATUS_POPOVER_ID} button {
  padding: 6px 12px; border-radius: 7px; border: none; font: inherit; cursor: pointer;
  background: var(--primary, #e5e5e5); color: var(--primary-foreground, #171717);
}
#${STATUS_POPOVER_ID} button.shiver-status-ghost {
  background: transparent; color: var(--muted-foreground, #a1a1a1);
}
`;

  const closePopover = () => document.getElementById(STATUS_POPOVER_ID)?.remove();

  const openPopover = (anchor: Element) => {
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

    note.className = 'shiver-status-note';
    note.hidden = true;

    clear.textContent = 'Clear';
    clear.className = 'shiver-status-ghost';
    save.textContent = 'Save';

    row.className = 'shiver-status-row';
    row.append(clear, save);
    host.append(label, input, note, row);

    // Shown before the current status is known, rather than after. Asking first meant a click did
    // nothing at all until the server answered — and if it never answered, nothing at all, ever.
    document.body.append(host);

    // above the button that opened it, which sits at the bottom of the window
    const box = anchor.getBoundingClientRect();
    const size = host.getBoundingClientRect();

    host.style.left = `${Math.max(8, Math.min(box.left, window.innerWidth - size.width - 8))}px`;
    host.style.top = `${Math.max(8, box.top - size.height - 8)}px`;

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

    const fail = (message: string) => {
      note.textContent = message;
      note.hidden = false;
      save.disabled = false;
      clear.disabled = false;
    };

    const commit = async (value: string) => {
      save.disabled = true;
      clear.disabled = true;
      note.hidden = true;

      const result = await callPlugin('setStatus', { status: value });

      // The relay answers with the status the server stored, so this is a real acknowledgement and
      // not a hope. Closing before it arrived would have hidden a failure behind a tidy animation.
      if (!result || typeof result.status !== 'string') {
        fail('Could not save that. The Shiver plugin may no longer be installed here.');

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

    // a click anywhere else closes it, the way Sharkord's own popovers behave
    setTimeout(() => {
      const dismiss = (event: MouseEvent) => {
        if (event.target instanceof Node && host.contains(event.target)) return;

        closePopover();
        document.removeEventListener('mousedown', dismiss, true);
      };

      document.addEventListener('mousedown', dismiss, true);
    }, 0);
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

      void openPopover(button);
    });

    gear.parentElement?.insertBefore(button, gear);
  };

  // The panel is rebuilt whenever the client re-renders it — a voice state change is enough — so
  // adding it once is not enough. `add` is cheap and returns immediately when the button is there.
  void waitForPlugin().then((present) => {
    if (!present) return;

    add();

    onDomSettled(add);
  });
}

/** Sharkord's compose editor. `TestId.MESSAGE_COMPOSE_EDITOR` in `packages/shared/src/test-ids.ts`. */
const COMPOSE_EDITOR = '[data-testid="message-compose-editor"]';

/** A file waiting to be sent, as drawn by `channel-view/text/preview-file.tsx`. */
const PENDING_FILE = 'div[class~="w-48"][class~="group"][class~="rounded-lg"]';

/**
 * Puts the caret back in the compose box after a file is attached, so Enter sends it.
 *
 * Attaching a picture and pressing Enter does nothing, and attaching a picture, typing a word and
 * pressing Enter sends both — which reads like Sharkord refusing to send a message with no text.
 * It is not. `handleSend` in `message-compose/index.tsx` accepts an empty message whenever there
 * are files; the Enter that would call it is bound inside the editor, through ProseMirror's
 * `handleKeyDown`. Picking a file moves focus to the paperclip and the file dialog, nothing ever
 * gives it back, and the keystroke is delivered to a button that does not listen. Typing first
 * hides the whole thing, because then the editor already had focus.
 *
 * Shiver puts the focus back rather than binding its own Enter: the editor is where the caret looks
 * like it should be, a caption typed after the picture then lands where it appears to, and no
 * second way of sending a message exists to disagree with Sharkord's own.
 *
 * Desktop only. On a phone, focusing the editor throws the keyboard up over the picture that was
 * just attached, and there is no Enter key waiting to be pressed there anyway.
 */
function installAttachmentFocus() {
  let pending = 0;

  /** somewhere the user is deliberately typing, which is never interrupted */
  const isTyping = (node: HTMLElement) =>
    node.isContentEditable || node.matches('input, textarea, select');

  const check = () => {
    const now = document.querySelectorAll(PENDING_FILE).length;
    const gained = now > pending;

    pending = now;

    if (!gained) return;

    const editor = document.querySelector<HTMLElement>(COMPOSE_EDITOR);

    if (!editor) return;

    const active = document.activeElement;

    if (active instanceof HTMLElement && active !== editor && isTyping(active)) return;

    editor.focus();

    // caret at the end, so a caption typed after the picture goes where it looks like it will
    const selection = window.getSelection();
    const range = document.createRange();

    range.selectNodeContents(editor);
    range.collapse(false);

    selection?.removeAllRanges();
    selection?.addRange(range);
  };

  onDomSettled(check);
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

  onDomSettled(paint);

  defineHook('__SHIVER_SET_ATTACHMENT_CARDS__', (next: boolean) => {
    on = next;
    paint();
  });
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

function whenDocumentReady(run: () => void) {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', run, { once: true });

    return;
  }

  run();
}

// Started last, deliberately. `install` assigns module-level `let` bindings declared further down
// this file, and calling it from the top put those in the temporal dead zone: the whole bridge threw
// before `__SHIVER_DRAIN__` was ever assigned, which silently took notifications, DMs and mute sync
// with it. Function declarations hoist, so invoking from the bottom is safe; a `let` does not.
const config = window.__SHIVER__;

// Taken out of the page as soon as it has been read.
//
// It carries this server's session token, which belongs to this origin — so leaving it there is not
// a cross-server leak. What it is, is a wider blast radius: any third-party script the server loads,
// or any injection into its page, could read the token off a global instead of having to reach into
// storage for it. The bridge is an `initialization_script` here, so it runs before the page's own
// scripts and nothing of the server's ever sees the global at all.
delete window.__SHIVER__;

/**
 * The bridge runs in the page, and only in the page.
 *
 * Whether an initialization script reaches cross-origin subframes is wry's business and differs by
 * platform — on WebView2 the underlying `AddScriptToExecuteOnDocumentCreated` is an all-frames API
 * by nature. If it does reach them, `seedAutoLogin` writes this server's session token into the
 * `localStorage` of every origin the page happens to embed.
 *
 * One comparison removes the dependency on someone else's implementation detail entirely, and a
 * subframe has nothing to do here in any case: the rail, the bell and the drain are all about the
 * document the user is looking at.
 */
const isTopFrame = (() => {
  try {
    return window.top === window.self;
  } catch {
    // a cross-origin parent makes `window.top` throw, which is itself the answer
    return false;
  }
})();

if (config && isTopFrame) {
  try {
    install(config);
  } catch (error) {
    // never let a bridge failure take the user's client down with it
    console.error('[shiver] bridge failed to install', error);
  }
}

export {};
