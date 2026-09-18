import { useCallback, useEffect, useState } from 'react';

import { api, errorMessage } from '../api';

import { HotkeyField } from './HotkeyField';
import { automaticTextColor } from '../theme';
import { playNotificationSound } from '../sounds';
import {
  DEFAULT_ACCENT_COLOR,
  DEFAULT_THEME_COLOR,
  MAX_PAGES_KEPT,
  MAX_SOUND_VOLUME,
  MIN_PAGES_KEPT,
  type Settings
} from '../types';

type Props = {
  settings: Settings;
  onSave: (settings: Settings) => void;
  onClose: () => void;
};

/**
 * The sections, in the order they are listed.
 *
 * Grouped by what a person came here to change rather than by what the code calls things. Shiver's
 * settings are few enough that one list was readable for a while, but it had grown to eight
 * unrelated controls in a column — the colours, a keyboard shortcut and a memory dial all in the
 * same run — and the only way to find anything was to read all of it.
 */
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

  const handleTheme = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => update('themeColor', event.target.value),
    [update]
  );

  const handleAccent = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => update('accentColor', event.target.value),
    [update]
  );

  const handleText = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => update('textColor', event.target.value),
    [update]
  );

  const handleVolume = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) =>
      update('soundVolume', Number(event.target.value)),
    [update]
  );

  // Nobody can judge 180% by reading it, and the setting is saved in another webview from the one
  // that plays the ping — so the panel plays it here, at the level currently under the thumb.
  const handleTestSound = useCallback(() => {
    playNotificationSound(draft.soundVolume);
  }, [draft.soundVolume]);

  const handleResetColors = useCallback(() => {
    setDraft((current) => ({
      ...current,
      themeColor: DEFAULT_THEME_COLOR,
      accentColor: DEFAULT_ACCENT_COLOR,
      textColor: null
    }));
  }, []);

  const isDefaultColors =
    draft.themeColor.toLowerCase() === DEFAULT_THEME_COLOR &&
    draft.accentColor.toLowerCase() === DEFAULT_ACCENT_COLOR &&
    draft.textColor === null;

  const handleSubmit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();

      onSave(draft);
    },
    [draft, onSave]
  );

  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState<string | null>(null);
  const [newer, setNewer] = useState<string | null>(null);

  const runCheck = useCallback(async () => {
    setChecking(true);
    setChecked(null);

    try {
      const found = await api.checkForUpdate();

      if (found) setNewer(found);
      else setChecked('You are on the newest version.');
    } catch (error) {
      setChecked(errorMessage(error));
    } finally {
      setChecking(false);
    }
  }, []);

  const handleInstall = useCallback(async () => {
    setChecked('Downloading…');

    try {
      await api.installUpdate();
    } catch (error) {
      // on success the installer takes over and this process ends, so only a failure comes back
      setChecked(errorMessage(error));
    }
  }, []);

  // Answers in place of a toast: this panel has nowhere to put one, and a button that says nothing
  // when pressed is exactly the sort of silence this feature exists to fix.
  const [permissionsReset, setPermissionsReset] = useState<string | null>(null);

  const handleResetPermissions = useCallback(async () => {
    try {
      const forgotten = await api.resetMediaPermissions();

      // The count, not a cheerful noise. This button spent a release saying "Forgotten" while
      // clearing nothing at all, and the only thing that would have caught it sooner is the number.
      setPermissionsReset(
        forgotten === 0
          ? 'Nothing was stored — no server has been answered yet'
          : `Forgotten for ${forgotten} ${forgotten === 1 ? 'answer' : 'answers'} — you will be asked again`
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
          {section === 'appearance' ? (
            <>
              <p className="hint">
                Applied to Shiver and to every server you open in it. On the defaults Shiver restyles
                nothing, so servers look exactly as they do in a browser.
              </p>

              <label className="field">
                <span>Background colour</span>
                <div className="field-row">
                  <input type="color" value={draft.themeColor} onChange={handleTheme} />
                  <input
                    type="text"
                    value={draft.themeColor}
                    onChange={handleTheme}
                    spellCheck={false}
                  />
                </div>
              </label>

              <label className="field">
                <span>Accent colour</span>
                <div className="field-row">
                  <input type="color" value={draft.accentColor} onChange={handleAccent} />
                  <input
                    type="text"
                    value={draft.accentColor}
                    onChange={handleAccent}
                    spellCheck={false}
                  />
                </div>
              </label>

              <label className="field">
                <span>Text colour</span>
                <div className="field-row">
                  <input
                    type="color"
                    value={draft.textColor ?? automaticTextColor(draft.themeColor)}
                    onChange={handleText}
                  />
                  <input
                    type="text"
                    value={draft.textColor ?? ''}
                    placeholder="automatic"
                    onChange={(event) => update('textColor', event.target.value || null)}
                    spellCheck={false}
                  />
                  <button
                    type="button"
                    className="ghost"
                    disabled={draft.textColor === null}
                    onClick={() => update('textColor', null)}
                  >
                    Automatic
                  </button>
                </div>
                <small className="hint">
                  Left automatic, text follows the background — dark on light, light on dark.
                </small>
              </label>

              <label className="checkbox">
                <input
                  type="checkbox"
                  checked={draft.minimiseAttachments}
                  onChange={(event) => update('minimiseAttachments', event.target.checked)}
                />
                <span>
                  Shrink the card under a picture
                  <small>
                    A posted image shows up twice, as the picture and as a card naming the file.
                    Documents and archives keep their card, since there it is all you have to go on.
                  </small>
                </span>
              </label>

              <div className="actions">
                <button
                  type="button"
                  className="ghost"
                  onClick={handleResetColors}
                  disabled={isDefaultColors}
                >
                  Reset colours
                </button>
              </div>
            </>
          ) : null}

          {section === 'notifications' ? (
            <>
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

              <label className="field">
                <span>Sound volume</span>
                <div className="field-row">
                  <input
                    type="range"
                    min={0}
                    max={MAX_SOUND_VOLUME}
                    step={5}
                    value={draft.soundVolume}
                    onChange={handleVolume}
                  />
                  <output className="volume-readout">{draft.soundVolume}%</output>
                  <button type="button" className="ghost" onClick={handleTestSound}>
                    Test
                  </button>
                </div>
                <small className="hint">
                  Moves Shiver's ping and each server's own sounds together. Goes to{' '}
                  {MAX_SOUND_VOLUME}%, for a ping that has to carry over a call. How loud people are
                  in a call is set per person, in the server's own controls.
                </small>
              </label>
            </>
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
            <label className="field">
              <span>Camera and microphone</span>
              <div className="field-row">
                <button type="button" className="ghost" onClick={handleResetPermissions}>
                  {permissionsReset ?? 'Ask me again'}
                </button>
              </div>
              <small className="hint">
                A server asks once and the webview remembers for ever, so one turned down by
                accident fails silently after that. This forgets those answers, for every server.
              </small>
            </label>
          ) : null}

          {section === 'about' ? (
            <>
              <p className="hint">
                {version ? `Shiver ${version}.` : 'Shiver.'} A multi-server client for Sharkord.
              </p>

              <div className="actions">
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void api.openRepository().catch(() => undefined)}
                >
                  View the project on GitHub
                </button>

                {newer ? (
                  <button type="button" className="primary" onClick={() => void handleInstall()}>
                    Install Shiver {newer}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="ghost"
                    disabled={checking}
                    onClick={() => void runCheck()}
                  >
                    {checking ? 'Checking…' : 'Check for updates'}
                  </button>
                )}
              </div>

              {/* "nothing newer" is a real answer and worth saying: a check that only ever speaks
                  up with good news leaves you wondering whether it ran at all */}
              {checked ? <p className="hint">{checked}</p> : null}
            </>
          ) : null}

          {section === 'servers' ? (
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
