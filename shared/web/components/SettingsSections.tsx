import { type ReactNode, useCallback, useEffect, useState } from 'react';

import { automaticTextColor } from '../colors';
import { errorMessage } from '../errors';
import { DEFAULT_ACCENT_COLOR, DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME } from '../settings';
import './ui.css';

/**
 * The settings sections both clients have. Each takes the fields it edits from the app's draft and
 * hands changes back; the app keeps its own section navigation and its Save.
 */

type Edit<T> = { draft: T; onChange: (patch: Partial<T>) => void };

export type AppearanceFields = {
  themeColor: string;
  accentColor: string;
  /** null takes it from the background */
  textColor: string | null;
  minimiseAttachments: boolean;
};

export const AppearanceSection = ({ draft, onChange }: Edit<AppearanceFields>) => {
  const isDefaultColors =
    draft.themeColor.toLowerCase() === DEFAULT_THEME_COLOR &&
    draft.accentColor.toLowerCase() === DEFAULT_ACCENT_COLOR &&
    draft.textColor === null;

  return (
    <>
      <p className="hint">
        Applied to Shiver and to every server you open in it. On the defaults Shiver restyles nothing, so servers look
        exactly as they do in a browser.
      </p>

      <ColorField label="Background colour" value={draft.themeColor} onChange={(themeColor) => onChange({ themeColor })} />
      <ColorField label="Accent colour" value={draft.accentColor} onChange={(accentColor) => onChange({ accentColor })} />

      <label className="field">
        <span>Text colour</span>
        <div className="field-row">
          <input
            type="color"
            value={draft.textColor ?? automaticTextColor(draft.themeColor)}
            onChange={(event) => onChange({ textColor: event.target.value })}
          />
          <input
            type="text"
            value={draft.textColor ?? ''}
            placeholder="automatic"
            onChange={(event) => onChange({ textColor: event.target.value || null })}
            spellCheck={false}
          />
          <button
            type="button"
            className="ghost"
            disabled={draft.textColor === null}
            onClick={() => onChange({ textColor: null })}
          >
            Automatic
          </button>
        </div>
        <small className="hint">Left automatic, text follows the background — dark on light, light on dark.</small>
      </label>

      <label className="checkbox">
        <input
          type="checkbox"
          checked={draft.minimiseAttachments}
          onChange={(event) => onChange({ minimiseAttachments: event.target.checked })}
        />
        <span>
          Shrink the card under a picture
          <small>
            A posted image shows up twice, as the picture and as a card naming the file. Documents and archives keep
            their card, since there it is all you have to go on.
          </small>
        </span>
      </label>

      <div className="actions">
        <button
          type="button"
          className="ghost"
          disabled={isDefaultColors}
          onClick={() =>
            onChange({ themeColor: DEFAULT_THEME_COLOR, accentColor: DEFAULT_ACCENT_COLOR, textColor: null })
          }
        >
          Reset colours
        </button>
      </div>
    </>
  );
};

const ColorField = ({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) => (
  <label className="field">
    <span>{label}</span>
    <div className="field-row">
      <input type="color" value={value} onChange={(event) => onChange(event.target.value)} />
      <input type="text" value={value} onChange={(event) => onChange(event.target.value)} spellCheck={false} />
    </div>
  </label>
);

type NotificationsProps = Edit<{ soundVolume: number }> & {
  /** what the volume moves on this platform, and when it takes effect */
  volumeHint: ReactNode;
  /** plays the ping at the level under the thumb, where Shiver plays one itself */
  onTest?: (volume: number) => void;
  /** this platform's own switches, above the volume */
  children?: ReactNode;
};

export const NotificationsSection = ({ draft, onChange, volumeHint, onTest, children }: NotificationsProps) => (
  <>
    {children}

    <label className="field">
      <span>Sound volume</span>
      <div className="field-row">
        <input
          type="range"
          min={0}
          max={MAX_SOUND_VOLUME}
          step={5}
          value={draft.soundVolume}
          onChange={(event) => onChange({ soundVolume: Number(event.target.value) })}
        />
        <output className="volume-readout">{draft.soundVolume}%</output>
        {/* Nobody can judge 180% by reading it, so the ping plays at the level under the thumb */}
        {onTest ? (
          <button type="button" className="ghost" onClick={() => onTest(draft.soundVolume)}>
            Test
          </button>
        ) : null}
      </div>
      <small className="hint">{volumeHint}</small>
    </label>
  </>
);

type AboutProps = {
  appVersion: () => Promise<string>;
  /** the version already found, if the core has found one */
  available: () => Promise<string | null>;
  checkForUpdate: () => Promise<string | null>;
  openRepository: () => Promise<void>;
  /** "Install Shiver 1.2" or "Get Shiver 1.2": the words before the version */
  takeLabel: string;
  /** said above the button, where taking it needs explaining */
  takeHint?: ReactNode;
  /** said while `onTake` runs, if it takes a while */
  takingNote?: string;
  /** on desktop the installer takes over and Shiver exits, so only a failure comes back */
  onTake: () => Promise<void>;
};

export const AboutSection = ({
  appVersion,
  available,
  checkForUpdate,
  openRepository,
  takeLabel,
  takeHint,
  takingNote,
  onTake
}: AboutProps) => {
  /** which build this is, because that is the first question about any bug */
  const [version, setVersion] = useState('');
  const [newer, setNewer] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState<string | null>(null);

  useEffect(() => {
    appVersion().then(setVersion, () => setVersion(''));
    available().then(setNewer, () => setNewer(null));
  }, [appVersion, available]);

  const runCheck = useCallback(async () => {
    setChecking(true);
    setChecked(null);

    try {
      const found = await checkForUpdate();

      if (found) setNewer(found);
      else setChecked('You are on the newest version.');
    } catch (error) {
      setChecked(errorMessage(error));
    } finally {
      setChecking(false);
    }
  }, [checkForUpdate]);

  const take = useCallback(async () => {
    setChecked(takingNote ?? null);

    try {
      await onTake();
    } catch (error) {
      setChecked(errorMessage(error));
    }
  }, [onTake, takingNote]);

  return (
    <>
      <p className="hint">{version ? `Shiver ${version}.` : 'Shiver.'} A multi-server client for Sharkord.</p>

      {newer && takeHint ? <p className="hint">{takeHint}</p> : null}

      <div className="actions">
        <button type="button" className="ghost" onClick={() => void openRepository().catch(() => undefined)}>
          View the project on GitHub
        </button>

        {newer ? (
          <button type="button" className="primary" onClick={() => void take()}>
            {takeLabel} {newer}
          </button>
        ) : (
          <button type="button" className="ghost" disabled={checking} onClick={() => void runCheck()}>
            {checking ? 'Checking…' : 'Check for updates'}
          </button>
        )}
      </div>

      {/* "nothing newer" is a real answer and worth saying: a check that only ever speaks up with good
          news leaves you wondering whether it ran at all */}
      {checked ? <p className="hint">{checked}</p> : null}
    </>
  );
};
