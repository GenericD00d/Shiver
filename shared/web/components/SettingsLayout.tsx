/**
 * Shiver's settings, laid out as Sharkord's own (its `SettingsLayout`): a bar with Back and the
 * title, the sections down one side with an icon each (a drawer behind a menu button where the
 * screen is narrow), and the section chosen as cards in a centred column. Changes wait in a draft
 * until the bar that slides up saves them, so leaving with some unsaved points at it instead.
 */

import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';

import { ChevronLeftIcon, MenuIcon } from './icons';
import './settings.css';

export type SettingsEntry<Id extends string> = {
  id: Id;
  label: string;
  icon: (props: { className?: string }) => ReactNode;
};

type LayoutProps<Id extends string> = {
  title: string;
  entries: readonly SettingsEntry<Id>[];
  selected: Id;
  onSelect: (id: Id) => void;
  onBack: () => void;
  /** above the cards: an error, the update notice */
  notices?: ReactNode;
  /** under the cards: the save bar */
  footer?: ReactNode;
  children: ReactNode;
};

export const SettingsLayout = <Id extends string>({
  title,
  entries,
  selected,
  onSelect,
  onBack,
  notices,
  footer,
  children
}: LayoutProps<Id>) => {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const main = useRef<HTMLElement>(null);

  // a new section starts at its top, as a new page would
  useEffect(() => {
    main.current?.scrollTo({ top: 0 });
  }, [selected]);

  return (
    <div className="settings-layout">
      <header className="settings-top">
        <button type="button" className="settings-icon-button" aria-label="Back" onClick={onBack}>
          <ChevronLeftIcon />
        </button>

        <h1>{title}</h1>

        <button
          type="button"
          className="settings-icon-button settings-menu"
          aria-label="Settings sections"
          aria-expanded={drawerOpen}
          onClick={() => setDrawerOpen((open) => !open)}
        >
          <MenuIcon />
        </button>
      </header>

      <div className="settings-frame">
        {drawerOpen ? <div className="settings-scrim" aria-hidden="true" onClick={() => setDrawerOpen(false)} /> : null}

        <nav className={drawerOpen ? 'settings-nav open' : 'settings-nav'} aria-label="Settings sections">
          {entries.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              className={id === selected ? 'settings-entry active' : 'settings-entry'}
              aria-current={id === selected}
              onClick={() => {
                setDrawerOpen(false);
                onSelect(id);
              }}
            >
              <Icon />
              <span>{label}</span>
            </button>
          ))}
        </nav>

        <main className="settings-main" ref={main}>
          <div className="settings-column">
            {notices}
            {children}
          </div>

          {footer}
        </main>
      </div>
    </div>
  );
};

/** A group of settings under a title, as Sharkord's `SettingsSection` card. */
export const SettingsCard = ({
  title,
  description,
  children
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) => (
  <section className="settings-card">
    <header>
      <h2>{title}</h2>
      {description ? <p>{description}</p> : null}
    </header>

    <div className="settings-card-body">{children}</div>
  </section>
);

/** One setting: its name, what it does, then its controls (Sharkord's `Group`). */
export const Setting = ({
  label,
  description,
  children
}: {
  label: string;
  description?: ReactNode;
  children: ReactNode;
}) => (
  <div className="setting">
    <div className="setting-text">
      <span className="setting-label">{label}</span>
      {description ? <span className="setting-description">{description}</span> : null}
    </div>

    <div className="setting-control">{children}</div>
  </div>
);

/** A setting that is on or off, with a switch under its name; the whole of it takes the press. */
export const Toggle = ({
  label,
  description,
  checked,
  onChange
}: {
  label: string;
  description?: ReactNode;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) => (
  <label className="setting">
    <span className="setting-text">
      <span className="setting-label">{label}</span>
      {description ? <span className="setting-description">{description}</span> : null}
    </span>

    <span className="setting-control">
      <input type="checkbox" role="switch" className="switch" checked={checked} onChange={(event) => onChange(event.target.checked)} />
    </span>
  </label>
);

/** How long the save bar shakes when leaving would lose what it holds. */
const NUDGE_MS = 600;

/**
 * The settings being edited, against what was saved last: `dirty` while they differ. `leave` runs
 * `go` only when nothing would be lost, and otherwise makes the save bar say so.
 */
export function useSettingsDraft<T>(saved: T, save: (draft: T) => Promise<boolean>) {
  const [draft, setDraft] = useState(saved);
  const [baseline, setBaseline] = useState(saved);
  const [saving, setSaving] = useState(false);
  const [nudged, setNudged] = useState(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);

  const patch = useCallback((changes: Partial<T>) => setDraft((current) => ({ ...current, ...changes })), []);

  const commit = useCallback(async () => {
    setSaving(true);

    try {
      if (await save(draft)) setBaseline(draft);
    } finally {
      setSaving(false);
    }
  }, [draft, save]);

  const discard = useCallback(() => setDraft(baseline), [baseline]);

  const leave = useCallback(
    (go: () => void) => {
      if (!dirty) return go();

      setNudged(true);
      window.setTimeout(() => setNudged(false), NUDGE_MS);
    },
    [dirty]
  );

  const bar = dirty ? (
    <div className="save-bar-dock">
      <div className={nudged ? 'save-bar nudged' : 'save-bar'} role="status">
        <span>{nudged ? 'Save or discard your changes first' : 'Unsaved changes'}</span>

        <div className="save-bar-actions">
          <button type="button" className="ghost" disabled={saving} onClick={discard}>
            Discard
          </button>
          <button type="button" className="primary" disabled={saving} onClick={() => void commit()}>
            {saving ? 'Saving…' : 'Save changes'}
          </button>
        </div>
      </div>
    </div>
  ) : null;

  return { draft, patch, leave, bar };
}
