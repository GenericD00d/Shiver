import { useCallback, useEffect, useState } from 'react';

import { api } from '../api';

import { LogFolder, PermissionsSection, ServersSection } from './DesktopSections';
import { HotkeyField } from './HotkeyField';
import { playNotificationSound } from '../sounds';
import { AboutSection, AppearanceSection, NotificationsSection } from '../../../shared/web/components/SettingsSections';
import { MAX_SOUND_VOLUME, type Settings } from '../types';

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

  // `settings` is not read again while the panel is open, so its count would stay as it was
  const [linksCleared, setLinksCleared] = useState(false);
  const clearLinks = useCallback(() => setLinksCleared(true), []);

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
            <PermissionsSection
              trustedLinks={linksCleared ? 0 : settings.trustedLinks.length}
              onLinksForgotten={clearLinks}
            />
          ) : null}

          {section === 'about' ? (
            <>
              <AboutSection
                appVersion={api.appVersion}
                available={api.availableUpdate}
                checkForUpdate={api.checkForUpdate}
                openRepository={api.openRepository}
                takeLabel="Install Shiver"
                takingNote="Downloading…"
                onTake={api.installUpdate}
              />
              <LogFolder />
            </>
          ) : null}

          {section === 'servers' ? <ServersSection draft={draft} update={update} /> : null}
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
