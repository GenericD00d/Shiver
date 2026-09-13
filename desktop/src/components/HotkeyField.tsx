import { useCallback, useState } from 'react';

type Props = {
  /** the shortcut in Tauri's accelerator form, or null for none */
  value: string | null;
  onChange: (value: string | null) => void;
};

/** Modifiers, in the order Tauri writes them, so a recorded shortcut round-trips unchanged. */
const MODIFIERS: [keyof Pick<KeyboardEvent, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>, string][] =
  [
    ['ctrlKey', 'Control'],
    ['altKey', 'Alt'],
    ['shiftKey', 'Shift'],
    ['metaKey', 'Super']
  ];

/** Tauri names the letter and digit keys by their character, and the rest by `KeyboardEvent.code`. */
const mainKey = (event: KeyboardEvent) => {
  if (/^Key[A-Z]$/.test(event.code)) return event.code.slice(3);
  if (/^Digit[0-9]$/.test(event.code)) return event.code.slice(5);
  if (/^F[0-9]{1,2}$/.test(event.code)) return event.code;

  const named: Record<string, string> = {
    Space: 'Space',
    Enter: 'Enter',
    Tab: 'Tab',
    Backspace: 'Backspace',
    Insert: 'Insert',
    Delete: 'Delete',
    Home: 'Home',
    End: 'End',
    PageUp: 'PageUp',
    PageDown: 'PageDown',
    ArrowUp: 'Up',
    ArrowDown: 'Down',
    ArrowLeft: 'Left',
    ArrowRight: 'Right'
  };

  return named[event.code] ?? null;
};

/** Whether the key pressed was only a modifier, which cannot be a shortcut on its own. */
const isModifierOnly = (event: KeyboardEvent) =>
  ['Control', 'Alt', 'Shift', 'Meta'].includes(event.key);

/**
 * Records a system-wide shortcut by listening for one.
 *
 * Typing an accelerator by hand means knowing how Tauri spells one, so the field takes the key
 * combination instead. It insists on a modifier: a shortcut is registered with the operating
 * system, so a bare letter would swallow that key in every other application on the machine.
 */
export const HotkeyField = ({ value, onChange }: Props) => {
  const [recording, setRecording] = useState(false);
  const [hint, setHint] = useState<string | null>(null);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();

      const native = event.nativeEvent;

      if (native.key === 'Escape') {
        setRecording(false);
        setHint(null);

        return;
      }

      if (isModifierOnly(native)) return;

      const parts = MODIFIERS.filter(([flag]) => native[flag]).map(([, name]) => name);
      const key = mainKey(native);

      if (!key) {
        setHint('That key cannot be used in a shortcut.');

        return;
      }

      if (parts.length === 0) {
        setHint('Add a modifier — Ctrl, Alt, Shift or Super.');

        return;
      }

      onChange([...parts, key].join('+'));
      setRecording(false);
      setHint(null);
    },
    [onChange]
  );

  return (
    <div className="hotkey">
      <button
        type="button"
        className={recording ? 'hotkey-input recording' : 'hotkey-input'}
        onClick={() => {
          setRecording(true);
          setHint(null);
        }}
        onBlur={() => setRecording(false)}
        onKeyDown={recording ? handleKeyDown : undefined}
      >
        {recording ? 'Press a combination…' : (value ?? 'Not set')}
      </button>

      <button
        type="button"
        className="ghost small"
        onClick={() => {
          onChange(null);
          setHint(null);
        }}
        disabled={!value}
      >
        Clear
      </button>

      {hint ? <small className="hotkey-hint">{hint}</small> : null}
    </div>
  );
};
