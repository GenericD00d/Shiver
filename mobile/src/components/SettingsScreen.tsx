import { useCallback, useEffect, useState } from 'react';

import { api, errorMessage } from '../api';
import { automaticTextColor } from '../../../shared/web/theme';
import { DEFAULT_ACCENT_COLOR, DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME, type Settings } from '../types';

/** Which group of settings to draw. The rest live in their own components — see `App`. */
export type SettingsSection = 'appearance' | 'notifications' | 'about';

type Props = {
  settings: Settings;
  onSave: (settings: Settings) => void;
  section: SettingsSection;
};

/**
 * One group of Shiver's own settings.
 *
 * Split by section rather than drawn as one long column: on a phone that column was most of a
 * screen's worth of scrolling to reach the volume slider, with the colours, an update offer and a
 * memory note in between. The sections themselves are listed by `App`, which is also where the
 * server list and the push settings live.
 */
export const SettingsScreen = ({ settings, onSave, section }: Props) => {
  const [draft, setDraft] = useState<Settings>(settings);
  const [version, setVersion] = useState('');
  const [newer, setNewer] = useState<string | null>(null);

  useEffect(() => {
    api.appVersion().then(setVersion, () => setVersion(''));
    api.updateAvailable().then(setNewer, () => setNewer(null));
  }, []);

  const update = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
    setDraft((current) => ({ ...current, [key]: value }));
  }, []);

  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState<string | null>(null);

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

  const isDefault =
    draft.themeColor.toLowerCase() === DEFAULT_THEME_COLOR &&
    draft.accentColor.toLowerCase() === DEFAULT_ACCENT_COLOR &&
    draft.textColor === null;

  if (section === 'about') {
    return (
      <div className="panel">
        <p className="hint">
          {version ? `Shiver ${version}.` : 'Shiver.'} A multi-server client for Sharkord.
        </p>

        <button
          type="button"
          className="ghost wide"
          onClick={() => void api.openRepository().catch(() => undefined)}
        >
          View the project on GitHub
        </button>

        {newer ? (
          <>
            <p className="hint">
              Shiver {newer} is available. Android will not let an app install itself, so this
              opens the download.
            </p>
            <button type="button" className="primary wide" onClick={() => void api.openReleases()}>
              Get Shiver {newer}
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="ghost wide"
              disabled={checking}
              onClick={() => void runCheck()}
            >
              {checking ? 'Checking…' : 'Check for updates'}
            </button>

            {/* "nothing newer" is a real answer and worth saying: a check that only ever speaks up
                with good news leaves you wondering whether it ran at all */}
            {checked ? <p className="hint">{checked}</p> : null}
          </>
        )}
      </div>
    );
  }

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();
        onSave(draft);
      }}
    >
      {section === 'appearance' ? (
        <>
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
              Left alone, text follows the background — dark on light, light on dark.
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
                Documents and archives keep theirs.
              </small>
            </span>
          </label>
        </>
      ) : null}

      {section === 'notifications' ? (
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
            Your servers' own sounds, up to {MAX_SOUND_VOLUME}% so a ping carries over a call.
            Takes effect next time you open a server.
          </small>
        </label>
      ) : null}

      <button type="submit" className="primary wide">
        Save
      </button>

      {section === 'appearance' ? (
        <>
          {draft.textColor ? (
            <button type="button" className="ghost wide" onClick={() => update('textColor', null)}>
              Back to automatic text
            </button>
          ) : null}

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
        </>
      ) : null}
    </form>
  );
};
