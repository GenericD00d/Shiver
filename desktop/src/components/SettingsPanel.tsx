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

export const SettingsPanel = ({ settings, onSave, onClose }: Props) => {
  const [draft, setDraft] = useState<Settings>(settings);
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

  const handleSounds = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) =>
      update('notificationSounds', event.target.checked),
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
    <form className="panel" onSubmit={handleSubmit}>
      <h1>Shiver settings{version ? ` — ${version}` : ''}</h1>
      <p className="hint">
        These apply to Shiver itself and to every server you open in it. On the default colours Shiver
        restyles nothing, so your servers look exactly as they do in a browser.
      </p>

      <label className="field">
        <span>Background colour</span>
        <div className="field-row">
          <input type="color" value={draft.themeColor} onChange={handleTheme} />
          <input type="text" value={draft.themeColor} onChange={handleTheme} spellCheck={false} />
        </div>
      </label>

      <label className="field">
        <span>Accent colour</span>
        <div className="field-row">
          <input type="color" value={draft.accentColor} onChange={handleAccent} />
          <input type="text" value={draft.accentColor} onChange={handleAccent} spellCheck={false} />
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
          Left automatic, text follows the background: dark on a light one, light on a dark one.
          Pick a colour and it is used everywhere instead — in Shiver and in every server you open.
        </small>
      </label>

      <label className="field">
        <span>Mute microphone shortcut</span>
        <HotkeyField
          value={draft.muteHotkey}
          onChange={(value) => update('muteHotkey', value)}
        />
        <small className="hint">
          Works anywhere, even when Shiver is not the window in front. It mutes the call you are in,
          on whichever server is holding it, and does nothing when you are not in one.
        </small>
      </label>

      <label className="checkbox">
        <input
          type="checkbox"
          checked={draft.notificationSounds}
          onChange={handleSounds}
        />
        <span>
          Play notification sounds
          <small>
            Shiver plays the ping itself, for the messages that get past your muted channels. Each
            server's own message ping is always silenced, whether this is on or off, because it
            sounds for every message and knows nothing about what you have muted. Voice and
            microphone sounds are left to the server.
          </small>
        </span>
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
            A posted image shows up twice: the picture, and a card under it with the filename and
            the size. This shrinks that card to its icon and moves the name into its tooltip. Files
            the server cannot show inline — documents, archives — keep their card in full, since
            there it is the only thing to go on.
          </small>
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
          How many servers keep their client running in the background. A loaded server switches to
          instantly; the rest have to start when you open them, which takes a moment. Every other
          setting here is a preference — this one is memory: a loaded server costs roughly a
          browser tab, so on a long rail this is most of what Shiver uses.
          <br />
          <strong>Your notifications are not affected.</strong> Shiver talks to every server you are
          not looking at directly, so unread badges, the bell and your conversations stay complete
          whatever this is set to.
        </small>
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
          Moves Shiver's own ping and each server's own sounds together — its notification tones and
          its join, leave, mute and camera clicks. Goes to {MAX_SOUND_VOLUME}%, which is louder than
          either would play on its own, for a ping that has to carry over a call. How loud people
          are in a call is not affected, and is set per person in the server's own controls.
        </small>
      </label>

      <label className="field">
        <span>Camera and microphone</span>
        <div className="field-row">
          <button type="button" className="ghost" onClick={handleResetPermissions}>
            {permissionsReset ?? 'Ask me again'}
          </button>
        </div>
        <small className="hint">
          A server asks once, and the webview remembers the answer for ever — so a camera or
          microphone turned down by accident just fails silently after that, with no way back from
          inside the page. This forgets those answers, for every server, and the next one to ask
          will ask you again. Sharing a screen is not listed because nothing is remembered about it:
          you are asked to pick a window every time, and saying no is only no to that one attempt.
        </small>
      </label>

      <div className="actions">
        <button type="submit" className="primary">
          Save
        </button>
        <button type="button" className="ghost" onClick={handleResetColors} disabled={isDefaultColors}>
          Reset colours
        </button>
        <button type="button" className="ghost" onClick={onClose}>
          Close
        </button>
      </div>
    </form>
  );
};
