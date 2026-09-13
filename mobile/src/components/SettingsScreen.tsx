import { useCallback, useEffect, useState } from 'react';

import { api } from '../api';

import { automaticTextColor } from '../theme';
import { DEFAULT_ACCENT_COLOR, DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME, type Settings } from '../types';

type Props = {
  settings: Settings;
  onSave: (settings: Settings) => void;
};

export const SettingsScreen = ({ settings, onSave }: Props) => {
  const [draft, setDraft] = useState<Settings>(settings);
  /**
   * Which build this is.
   *
   * Here because the first question about any bug is "which version are you on", and until now
   * neither of us could answer it — the phone gives no way to see it short of Android's app info.
   */
  const [version, setVersion] = useState('');

  useEffect(() => {
    api.appVersion().then(setVersion, () => setVersion(''));
  }, []);

  const update = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
    setDraft((current) => ({ ...current, [key]: value }));
  }, []);

  const isDefault =
    draft.themeColor.toLowerCase() === DEFAULT_THEME_COLOR &&
    draft.accentColor.toLowerCase() === DEFAULT_ACCENT_COLOR &&
    draft.textColor === null;

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();
        onSave(draft);
      }}
    >
      <p className="hint">
        {version ? `Shiver ${version}. ` : ''}These apply to Shiver and to every server you open in it. On the default colours Shiver restyles
        nothing, so your servers look exactly as they do in a browser.
      </p>

      <label className="field">
        <span>Background colour</span>
        <input
          type="color"
          value={draft.themeColor}
          onChange={(event) => update('themeColor', event.target.value)}
        />
      </label>

      <label className="field">
        <span>Accent colour</span>
        <input
          type="color"
          value={draft.accentColor}
          onChange={(event) => update('accentColor', event.target.value)}
        />
      </label>

      <label className="field">
        <span>Text colour</span>
        <input
          type="color"
          value={draft.textColor ?? automaticTextColor(draft.themeColor)}
          onChange={(event) => update('textColor', event.target.value)}
        />
        <small className="hint">
          Left alone, text follows the background: dark on a light one, light on a dark one. Pick a
          colour and it is used everywhere instead — in Shiver and in every server you open.
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
            A posted image shows up twice: the picture, and a card under it with the filename and
            the size. This shrinks that card to its icon. Files the server cannot show inline —
            documents, archives — keep their card in full, since there it is the only thing to go
            on.
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
            onChange={(event) => update('soundVolume', Number(event.target.value))}
          />
          <output className="volume-readout">{draft.soundVolume}%</output>
        </div>
        <small className="hint">
          How loud your servers' own sounds are — their notification tones and their join, leave,
          mute and camera clicks. Goes to {MAX_SOUND_VOLUME}%, past what they would play on their
          own, for a ping that has to carry over a call. Your phone's volume keys move everything at
          once; this moves the sounds against the call. How loud people are in a call is not
          affected, and takes effect the next time you open a server.
        </small>
      </label>

      {draft.textColor ? (
        <button type="button" className="ghost wide" onClick={() => update('textColor', null)}>
          Back to automatic text
        </button>
      ) : null}

      <button type="submit" className="primary wide">
        Save
      </button>

      <button
        type="button"
        className="ghost wide"
        disabled={isDefault}
        onClick={() =>
          setDraft((current) => ({
            ...current,
            themeColor: DEFAULT_THEME_COLOR,
            accentColor: DEFAULT_ACCENT_COLOR,
            textColor: null
          }))
        }
      >
        Reset colours
      </button>
    </form>
  );
};
