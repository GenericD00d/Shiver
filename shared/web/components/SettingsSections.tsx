import { type ReactNode, useCallback, useEffect, useState } from 'react';

import { automaticTextColor, liftHex } from '../colors';
import { errorMessage } from '../errors';
import {
  BUILT_IN_PRESETS,
  type ColorPreset,
  type Colors,
  MAX_COLOR_PRESETS,
  MAX_PRESET_NAME,
  MAX_SOUND_VOLUME,
  sameColors
} from '../settings';
import { accentTextColor } from '../theme';
import './ui.css';

/**
 * The settings sections both clients have. Each takes the fields it edits from the app's draft and
 * hands changes back; the app keeps its own section navigation and its Save.
 */

type Edit<T> = { draft: T; onChange: (patch: Partial<T>) => void };

export type AppearanceFields = Colors & {
  colorPresets: ColorPreset[];
  minimiseAttachments: boolean;
};

const colorsOf = ({ themeColor, accentColor, textColor, accentTextColor, sidebarColor }: Colors): Colors => ({
  themeColor,
  accentColor,
  textColor,
  accentTextColor,
  sidebarColor
});

export const AppearanceSection = ({ draft, onChange }: Edit<AppearanceFields>) => {
  const [presetName, setPresetName] = useState('');
  const name = presetName.trim();
  const isBuiltInName = BUILT_IN_PRESETS.some((preset) => preset.name.toLowerCase() === name.toLowerCase());
  const replaces = draft.colorPresets.some((preset) => preset.name === name);
  const isFull = draft.colorPresets.length >= MAX_COLOR_PRESETS && !replaces;

  const savePreset = () => {
    const kept = draft.colorPresets.filter((preset) => preset.name !== name);

    onChange({ colorPresets: [...kept, { name, ...colorsOf(draft) }] });
    setPresetName('');
  };

  return (
    <>
      <p className="hint">
        Applied to Shiver and to every server you open in it. On Sharkord&apos;s own colours Shiver restyles nothing, so
        servers look exactly as they do in a browser.
      </p>

      <div className="field">
        <span>Presets</span>
        <div className="presets">
          {BUILT_IN_PRESETS.map((preset) => (
            <PresetChip key={`built-in:${preset.name}`} preset={preset} draft={draft} onChange={onChange} />
          ))}
          {draft.colorPresets.map((preset) => (
            <PresetChip
              key={`saved:${preset.name}`}
              preset={preset}
              draft={draft}
              onChange={onChange}
              onRemove={() =>
                onChange({ colorPresets: draft.colorPresets.filter((saved) => saved.name !== preset.name) })
              }
            />
          ))}
        </div>
      </div>

      <ColorField label="Background colour" value={draft.themeColor} onChange={(themeColor) => onChange({ themeColor })} />
      <ColorField label="Accent colour" value={draft.accentColor} onChange={(accentColor) => onChange({ accentColor })} />

      <OptionalColorField
        label="Sidebar colour"
        value={draft.sidebarColor}
        automatic={liftHex(draft.themeColor, 90)}
        onChange={(sidebarColor) => onChange({ sidebarColor })}
        hint="The channel list, the member list and the bars along the top, and Shiver's server rail. Left automatic, a shade off the background."
      />

      <OptionalColorField
        label="Text colour"
        value={draft.textColor}
        automatic={automaticTextColor(draft.themeColor)}
        onChange={(textColor) => onChange({ textColor })}
        hint="Left automatic, text follows the background — dark on light, light on dark."
      />

      <OptionalColorField
        label="Accent text colour"
        value={draft.accentTextColor}
        automatic={accentTextColor(draft)}
        onChange={(accentTextColor) => onChange({ accentTextColor })}
        hint="Times and dates, links, mentions and Shiver's own icons. Left automatic, it is the accent, lightened or darkened where it would not read on the background."
      />

      <label className="field">
        <span>Save these colours as a preset</span>
        <div className="field-row">
          <input
            type="text"
            value={presetName}
            placeholder="Name"
            maxLength={MAX_PRESET_NAME}
            onChange={(event) => setPresetName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && name && !isBuiltInName && !isFull) {
                event.preventDefault();
                savePreset();
              }
            }}
            spellCheck={false}
          />
          <button type="button" className="ghost" disabled={!name || isBuiltInName || isFull} onClick={savePreset}>
            {replaces ? 'Replace' : 'Save'}
          </button>
        </div>
        <small className="hint">
          {isBuiltInName
            ? 'That name belongs to a preset Shiver comes with.'
            : isFull
              ? `Shiver keeps up to ${MAX_COLOR_PRESETS} presets; remove one to save another.`
              : 'Kept with your settings once you save them.'}
        </small>
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
    </>
  );
};

const PresetChip = ({
  preset,
  draft,
  onChange,
  onRemove
}: Edit<AppearanceFields> & { preset: ColorPreset; onRemove?: () => void }) => (
  <span className="preset">
    <button
      type="button"
      className={sameColors(draft, preset) ? 'preset-chip active' : 'preset-chip'}
      title={onRemove ? preset.name : `${preset.name} (comes with Shiver)`}
      onClick={() => onChange(colorsOf(preset))}
    >
      <span
        className="preset-swatch"
        aria-hidden="true"
        style={{ background: `linear-gradient(135deg, ${preset.themeColor} 50%, ${preset.accentColor} 50%)` }}
      />
      {preset.name}
    </button>
    {onRemove ? (
      <button type="button" className="preset-remove" aria-label={`Remove ${preset.name}`} onClick={onRemove}>
        ×
      </button>
    ) : null}
  </span>
);

const ColorField = ({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) => (
  <label className="field">
    <span>{label}</span>
    <div className="field-row">
      <input type="color" value={value} onChange={(event) => onChange(event.target.value)} />
      <input type="text" value={value} onChange={(event) => onChange(event.target.value)} spellCheck={false} />
    </div>
  </label>
);

/** A colour the user may leave to Shiver: empty is automatic, which the swatch shows. */
const OptionalColorField = ({
  label,
  value,
  automatic,
  onChange,
  hint
}: {
  label: string;
  value: string | null;
  automatic: string;
  onChange: (value: string | null) => void;
  hint: string;
}) => (
  <label className="field">
    <span>{label}</span>
    <div className="field-row">
      <input type="color" value={value ?? automatic} onChange={(event) => onChange(event.target.value)} />
      <input
        type="text"
        value={value ?? ''}
        placeholder="automatic"
        onChange={(event) => onChange(event.target.value || null)}
        spellCheck={false}
      />
      <button type="button" className="ghost" disabled={value === null} onClick={() => onChange(null)}>
        Automatic
      </button>
    </div>
    <small className="hint">{hint}</small>
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
  // a second click while the first download runs would fetch and start the installer twice
  const [taking, setTaking] = useState(false);

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
    setTaking(true);
    setChecked(takingNote ?? null);

    try {
      await onTake();
    } catch (error) {
      setChecked(errorMessage(error));
    } finally {
      setTaking(false);
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
          <button type="button" className="primary" disabled={taking} onClick={() => void take()}>
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
