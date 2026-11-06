/** The settings sections only the desktop client has, each a card with the state its own buttons need. */

import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';
import { MAX_PAGES_KEPT, MIN_PAGES_KEPT, type Settings } from '../types';
import { HotkeyField } from './HotkeyField';
import { Setting, SettingsCard, Toggle } from '../../../shared/web/components/SettingsLayout';

/** How the rail shortcuts read on this system (`CommandOrControl` in hotkey.rs). */
const IS_MAC = navigator.userAgent.includes('Mac');
const SHORTCUT_KEY = IS_MAC ? '⌘' : 'Ctrl';
const ALT_KEY = IS_MAC ? '⌥' : 'Alt';

/** Only on Windows does Shiver ask before a server uses the camera and microphone. */
const ASKS_FOR_MEDIA = navigator.userAgent.includes('Windows');

type Edit = {
  draft: Settings;
  onChange: (changes: Partial<Settings>) => void;
};

/** A button whose result replaces its label, for a screen with nowhere to put a toast. */
function useResult(run: () => Promise<string>) {
  const [result, setResult] = useState<string | null>(null);
  const handle = useCallback(async () => {
    try {
      setResult(await run());
    } catch (error) {
      setResult(errorMessage(error));
    }
  }, [run]);

  return [result, handle] as const;
}

/** Shiver's own two switches, which go above the shared volume. */
export const DesktopNotifications = ({ draft, onChange }: Edit) => (
  <>
    <Toggle
      label="Show system notifications"
      description="While Shiver is not the window in front, what reaches the bell also shows as a notification from your system. A direct message flashes Shiver in the taskbar either way."
      checked={draft.desktopNotifications}
      onChange={(desktopNotifications) => onChange({ desktopNotifications })}
    />

    <Toggle
      label="Play notification sounds"
      description="Shiver plays the ping itself, for messages that get past your muted channels. Each server's own message ping is always silenced, since it knows nothing about what you have muted."
      checked={draft.notificationSounds}
      onChange={(notificationSounds) => onChange({ notificationSounds })}
    />
  </>
);

export const VoiceSection = ({ draft, onChange }: Edit) => (
  <SettingsCard title="Voice" description="Your microphone in whichever call you are in, from anywhere.">
    <Setting
      label="Mute microphone shortcut"
      description="Works anywhere, even when Shiver is not the window in front. It mutes the call you are in, on whichever server is holding it."
    >
      <HotkeyField value={draft.muteHotkey} onChange={(muteHotkey) => onChange({ muteHotkey })} />
    </Setting>
  </SettingsCard>
);

type PermissionsProps = {
  /** how many sites open without asking */
  trustedLinks: number;
  onLinksForgotten: () => void;
};

export const PermissionsSection = ({ trustedLinks, onLinksForgotten }: PermissionsProps) => {
  const [permissionsReset, resetPermissions] = useResult(
    useCallback(async () => {
      const forgotten = await api.resetMediaPermissions();

      return `Done — ${forgotten} ${forgotten === 1 ? 'server asks' : 'servers ask'} again`;
    }, [])
  );
  const [linksForgotten, forgetLinks] = useResult(
    useCallback(async () => {
      await api.forgetTrustedLinks();
      onLinksForgotten();

      return 'Done — every link asks again';
    }, [onLinksForgotten])
  );

  return (
    <SettingsCard title="Permissions" description="What a server's page may do without asking you first.">
      {ASKS_FOR_MEDIA ? (
        <Setting
          label="Camera and microphone"
          description="Shiver asks before a server on screen uses them, and remembers a yes until you log out of that server. This forgets every yes, so each server asks again."
        >
          <div className="field-row">
            <button type="button" className="ghost" onClick={resetPermissions}>
              {permissionsReset ?? 'Ask me again'}
            </button>
          </div>
        </Setting>
      ) : null}

      <Setting
        label="Links from servers"
        description={`Shiver asks before opening a link a server's page wants opened, since a page can claim a click that never happened. A site set to open without asking does so only from the server it was set on. Set so far: ${trustedLinks}.`}
      >
        <div className="field-row">
          <button type="button" className="ghost" disabled={trustedLinks === 0} onClick={forgetLinks}>
            {linksForgotten ?? 'Always ask again'}
          </button>
        </div>
      </Setting>
    </SettingsCard>
  );
};

export const ServersSection = ({ draft, onChange }: Edit) => (
  <SettingsCard
    title="Servers"
    description={`${SHORTCUT_KEY}+1 to ${SHORTCUT_KEY}+9 open the rail's servers in order, and ${SHORTCUT_KEY}+${ALT_KEY}+↑ or ↓ the one above or below, while Shiver is in front.`}
  >
    <Toggle
      label="Check spelling as you type"
      description="Underlines misspelled words in a server's text boxes. Turn it off if you write in a language Windows has no dictionary for."
      checked={draft.spellcheck}
      onChange={(spellcheck) => onChange({ spellcheck })}
    />

    <Toggle
      label="Close to the tray"
      description="Closing the window leaves Shiver running behind an icon in the tray, so servers stay connected and notifications keep coming. Quit from the icon's menu."
      checked={draft.closeToTray}
      onChange={(closeToTray) => onChange({ closeToTray })}
    />

    <Toggle
      label="Start Shiver when you sign in"
      description="It starts minimised, or in the tray when it closes there."
      checked={draft.startAtLogin}
      onChange={(startAtLogin) => onChange({ startAtLogin })}
    />

    <Setting
      label="Servers kept loaded"
      description={
        <>
          How many servers keep a client running. A loaded one switches to instantly; the rest start when you open them.
          This is memory rather than taste — roughly a browser tab each. <strong>Notifications are unaffected.</strong>
        </>
      }
    >
      <div className="field-row">
        <input
          type="range"
          aria-label="Servers kept loaded"
          min={MIN_PAGES_KEPT}
          max={MAX_PAGES_KEPT}
          step={1}
          value={draft.pagesKept}
          onChange={(event) => onChange({ pagesKept: Number(event.target.value) })}
        />
        <output className="volume-readout">{draft.pagesKept}</output>
      </div>
    </Setting>
  </SettingsCard>
);

/** The log file's folder, under About. */
export const LogFolder = () => {
  const [error, setError] = useState<string | null>(null);

  return (
    <SettingsCard title="Log file" description="What Shiver and each server's page ran into, kept on this computer only.">
      <Setting label="Logs folder" description={<>Attach <code>shiver.log</code> when you report a problem.</>}>
        <div className="field-row">
          <button
            type="button"
            className="ghost"
            onClick={() =>
              void api.openLogsFolder().then(
                () => setError(null),
                (cause) => setError(errorMessage(cause))
              )
            }
          >
            {error ?? 'Open logs folder'}
          </button>
        </div>
      </Setting>
    </SettingsCard>
  );
};
