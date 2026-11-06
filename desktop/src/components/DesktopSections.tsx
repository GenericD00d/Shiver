/** The settings sections only the desktop client has, each with the state its own buttons need. */

import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';
import { MAX_PAGES_KEPT, MIN_PAGES_KEPT, type Settings } from '../types';

/** How the rail shortcuts read on this system (`CommandOrControl` in hotkey.rs). */
const IS_MAC = navigator.userAgent.includes('Mac');
const SHORTCUT_KEY = IS_MAC ? '⌘' : 'Ctrl';
const ALT_KEY = IS_MAC ? '⌥' : 'Alt';

/** Only on Windows does Shiver ask before a server uses the camera and microphone. */
const ASKS_FOR_MEDIA = navigator.userAgent.includes('Windows');

/** A button whose result replaces its label, for a panel with nowhere to put a toast. */
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
    <>
      {ASKS_FOR_MEDIA ? (
        <label className="field">
          <span>Camera and microphone</span>
          <div className="field-row">
            <button type="button" className="ghost" onClick={resetPermissions}>
              {permissionsReset ?? 'Ask me again'}
            </button>
          </div>
          <small className="hint">
            Shiver asks before a server on screen uses them, and remembers a yes until you log out of
            that server. This forgets every yes, so each server asks again.
          </small>
        </label>
      ) : null}

      <label className="field">
        <span>Links from servers</span>
        <div className="field-row">
          <button type="button" className="ghost" disabled={trustedLinks === 0} onClick={forgetLinks}>
            {linksForgotten ?? 'Always ask again'}
          </button>
        </div>
        <small className="hint">
          Shiver asks before opening a link a server's page wants opened, since a page can claim a
          click that never happened. A site set to open without asking does so only from the server
          it was set on. Set so far: {trustedLinks}.
        </small>
      </label>
    </>
  );
};

type ServersProps = {
  draft: Settings;
  update: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
};

export const ServersSection = ({ draft, update }: ServersProps) => (
  <>
    <p className="hint">
      {SHORTCUT_KEY}+1 to {SHORTCUT_KEY}+9 open the rail's servers in order, and {SHORTCUT_KEY}+
      {ALT_KEY}+↑ or ↓ the one above or below, while Shiver is in front.
    </p>

    <label className="checkbox">
      <input type="checkbox" checked={draft.spellcheck} onChange={(event) => update('spellcheck', event.target.checked)} />
      <span>
        Check spelling as you type
        <small>
          Underlines misspelled words in a server's text boxes. Turn it off if you write in a language
          Windows has no dictionary for.
        </small>
      </span>
    </label>

    <label className="checkbox">
      <input type="checkbox" checked={draft.closeToTray} onChange={(event) => update('closeToTray', event.target.checked)} />
      <span>
        Close to the tray
        <small>
          Closing the window leaves Shiver running behind an icon in the tray, so servers stay
          connected and notifications keep coming. Quit from the icon's menu.
        </small>
      </span>
    </label>

    <label className="checkbox">
      <input type="checkbox" checked={draft.startAtLogin} onChange={(event) => update('startAtLogin', event.target.checked)} />
      <span>
        Start Shiver when you sign in
        <small>It starts minimised, or in the tray when it closes there.</small>
      </span>
    </label>

    <label className="field">
      <span>Servers kept loaded</span>
      <div className="field-row">
        <input
          type="range"
          min={MIN_PAGES_KEPT}
          max={MAX_PAGES_KEPT}
          step={1}
          value={draft.pagesKept}
          onChange={(event) => update('pagesKept', Number(event.target.value))}
        />
        <output className="volume-readout">{draft.pagesKept}</output>
      </div>
      <small className="hint">
        How many servers keep a client running. A loaded one switches to instantly; the rest start
        when you open them. This is memory rather than taste — roughly a browser tab each.{' '}
        <strong>Notifications are unaffected.</strong>
      </small>
    </label>
  </>
);

/** The log file's folder, under About. */
export const LogFolder = () => {
  const [error, setError] = useState<string | null>(null);

  return (
    <label className="field">
      <span>Log file</span>
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
      <small className="hint">
        What Shiver and each server's page ran into, kept on this computer only. Attach <code>shiver.log</code>{' '}
        when you report a problem.
      </small>
    </label>
  );
};
