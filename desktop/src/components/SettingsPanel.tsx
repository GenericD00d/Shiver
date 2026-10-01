import { useCallback, useEffect, useState } from 'react';

import { api, errorMessage } from '../api';

import { HotkeyField } from './HotkeyField';
import { playNotificationSound } from '../sounds';
import { AboutSection, AppearanceSection, NotificationsSection } from '../../../shared/web/components/SettingsSections';
import { MAX_PAGES_KEPT, MAX_SOUND_VOLUME, MIN_PAGES_KEPT, type Settings } from '../types';

type Props = {
  settings: Settings;
  onSave: (settings: Settings) => void;
  onClose: () => void;
};

/** The settings sections, grouped by what the user came to change. */
const SECTIONS = [
  { id: 'appearance', label: 'Appearance' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'voice', label: 'Voice' },
  { id: 'permissions', label: 'Permissions' },
  { id: 'servers', label: 'Servers' },
  { id: 'about', label: 'About' }
] as const;

type SectionId = (typeof SECTIONS)[number]['id'];

/** How the rail shortcuts read on this system (`CommandOrControl` in hotkey.rs). */
const IS_MAC = navigator.userAgent.includes('Mac');
const SHORTCUT_KEY = IS_MAC ? '⌘' : 'Ctrl';
const ALT_KEY = IS_MAC ? '⌥' : 'Alt';

/** Only on Windows does Shiver ask before a server uses the camera and microphone. */
const ASKS_FOR_MEDIA = navigator.userAgent.includes('Windows');

export const SettingsPanel = ({ settings, onSave, onClose }: Props) => {
  const [draft, setDraft] = useState<Settings>(settings);
  const [section, setSection] = useState<SectionId>('appearance');
  /** which build this is, because that is the first question about any bug */
  const [version, setVersion] = useState('');

  useEffect(() => {
    api.appVersion().then(setVersion, () => setVersion(''));
  }, []);

  const update = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
    setDraft((current) => ({ ...current, [key]: value }));
  }, []);

  const patch = useCallback((changes: Partial<Settings>) => {
    setDraft((current) => ({ ...current, ...changes }));
  }, []);

  const handleSubmit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();

      onSave(draft);
    },
    [draft, onSave]
  );

  // the result is shown inline, since this panel has nowhere to put a toast
  const [permissionsReset, setPermissionsReset] = useState<string | null>(null);
  const [linksForgotten, setLinksForgotten] = useState<string | null>(null);

  const handleForgetLinks = useCallback(async () => {
    try {
      await api.forgetTrustedLinks();
      setLinksForgotten('Done — every link asks again');
    } catch (error) {
      setLinksForgotten(errorMessage(error));
    }
  }, []);

  const handleResetPermissions = useCallback(async () => {
    try {
      const forgotten = await api.resetMediaPermissions();

      setPermissionsReset(
        `Done — ${forgotten} ${forgotten === 1 ? 'server asks' : 'servers ask'} again`
      );
    } catch (error) {
      setPermissionsReset(errorMessage(error));
    }
  }, []);

  return (
    <form className="panel settings" onSubmit={handleSubmit}>
      <h1>Shiver settings{version ? ` — ${version}` : ''}</h1>

      <div className="settings-body">
        <nav className="settings-sections" aria-label="Settings sections">
          {SECTIONS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={entry.id === section ? 'settings-section active' : 'settings-section'}
              aria-current={entry.id === section}
              onClick={() => setSection(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </nav>

        <div className="settings-content">
          {section === 'appearance' ? <AppearanceSection draft={draft} onChange={patch} /> : null}

          {section === 'notifications' ? (
            <NotificationsSection
              draft={draft}
              onChange={patch}
              // the setting is saved in another webview from the one that plays the ping, so it plays here
              onTest={playNotificationSound}
              volumeHint={
                <>
                  Moves Shiver's ping and each server's own sounds together. Goes to {MAX_SOUND_VOLUME}%, for a ping
                  that has to carry over a call. How loud people are in a call is set per person, in the server's own
                  controls.
                </>
              }
            >
              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.desktopNotifications}
                  onChange={(event) => update('desktopNotifications', event.target.checked)}
                />
                <span>
                  Show system notifications
                  <small>
                    While Shiver is not the window in front, what reaches the bell also shows as a
                    notification from your system. A direct message flashes Shiver in the taskbar
                    either way.
                  </small>
                </span>
              </label>

              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.notificationSounds}
                  onChange={(event) => update('notificationSounds', event.target.checked)}
                />
                <span>
                  Play notification sounds
                  <small>
                    Shiver plays the ping itself, for messages that get past your muted channels.
                    Each server's own message ping is always silenced, since it knows nothing about
                    what you have muted.
                  </small>
                </span>
              </label>
            </NotificationsSection>
          ) : null}

          {section === 'voice' ? (
            <label className="field">
              <span>Mute microphone shortcut</span>
              <HotkeyField value={draft.muteHotkey} onChange={(value) => update('muteHotkey', value)} />
              <small className="hint">
                Works anywhere, even when Shiver is not the window in front. It mutes the call you
                are in, on whichever server is holding it.
              </small>
            </label>
          ) : null}

          {section === 'permissions' ? (
            <>
              {ASKS_FOR_MEDIA ? (
                <label className="field">
                  <span>Camera and microphone</span>
                  <div className="field-row">
                    <button type="button" className="ghost" onClick={handleResetPermissions}>
                      {permissionsReset ?? 'Ask me again'}
                    </button>
                  </div>
                  <small className="hint">
                    Shiver asks before a server on screen uses them, and remembers a yes until you
                    log out of that server. This forgets every yes, so each server asks again.
                  </small>
                </label>
              ) : null}

              <label className="field">
                <span>Links from servers</span>
                <div className="field-row">
                  <button
                    type="button"
                    className="ghost"
                    disabled={settings.trustedLinks.length === 0}
                    onClick={handleForgetLinks}
                  >
                    {linksForgotten ?? 'Always ask again'}
                  </button>
                </div>
                <small className="hint">
                  Shiver asks before opening a link a server's page wants opened, since a page can
                  claim a click that never happened. A site set to open without asking does so only
                  from the server it was set on. Set so far: {settings.trustedLinks.length}.
                </small>
              </label>
            </>
          ) : null}

          {section === 'about' ? (
            <AboutSection
              appVersion={api.appVersion}
              available={api.availableUpdate}
              checkForUpdate={api.checkForUpdate}
              openRepository={api.openRepository}
              takeLabel="Install Shiver"
              takingNote="Downloading…"
              onTake={api.installUpdate}
            />
          ) : null}

          {section === 'servers' ? (
            <>
              <p className="hint">
                {SHORTCUT_KEY}+1 to {SHORTCUT_KEY}+9 open the rail's servers in order, and{' '}
                {SHORTCUT_KEY}+{ALT_KEY}+↑ or ↓ the one above or below, while Shiver is in front.
              </p>

              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.closeToTray}
                  onChange={(event) => update('closeToTray', event.target.checked)}
                />
                <span>
                  Close to the tray
                  <small>
                    Closing the window leaves Shiver running behind an icon in the tray, so servers
                    stay connected and notifications keep coming. Quit from the icon's menu.
                  </small>
                </span>
              </label>

              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.startAtLogin}
                  onChange={(event) => update('startAtLogin', event.target.checked)}
                />
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
                  How many servers keep a client running. A loaded one switches to instantly; the rest
                  start when you open them. This is memory rather than taste — roughly a browser tab
                  each. <strong>Notifications are unaffected.</strong>
                </small>
              </label>
            </>
          ) : null}
        </div>
      </div>

      <div className="actions">
        <button type="submit" className="primary">
          Save
        </button>
        <button type="button" className="ghost" onClick={onClose}>
          Close
        </button>
      </div>
    </form>
  );
};
