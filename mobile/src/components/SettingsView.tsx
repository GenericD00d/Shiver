import { type ReactNode, useState } from 'react';

import { api } from '../api';
import type { ConfirmAction } from './ActionConfirm';
import { BackgroundNotifications } from './BackgroundNotifications';
import { ServerList } from './ServerList';
import { Sessions, TrustedLinks } from './Sessions';
import { type SettingsSection, SettingsScreen } from './SettingsScreen';
import type { Registry, ServerEntry, SessionStates, Settings } from '../types';

/** The settings sections, in the order they are listed. */
const SECTIONS: { id: SettingsSection | 'servers'; label: string }[] = [
  { id: 'appearance', label: 'Appearance' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'servers', label: 'Servers' },
  { id: 'about', label: 'About' }
];

type Props = {
  registry: Registry;
  /** in rail order, with their logos */
  servers: ServerEntry[];
  sessions: SessionStates;
  /** reads the session states again */
  onSessionsChanged: () => void;
  onSave: (settings: Settings) => void;
  /** runs a change, then reads everything again */
  change: (run: () => Promise<unknown>) => Promise<void>;
  onOpen: (entryId: string) => void;
  onAsk: (action: ConfirmAction, entryId: string) => void;
  onAdd: () => void;
  onSignIn: (entryId: string) => void;
  onBack: () => void;
  /** above the sections: an error, the update notice */
  notices?: ReactNode;
};

/**
 * Shiver's settings: the bar across the top, the sections in a drawer over a scrim (as Sharkord's
 * own are on a phone), and the one chosen. Drawn in Shiver's page and over a server's page alike.
 */
export const SettingsView = ({
  registry,
  servers,
  sessions,
  onSessionsChanged,
  onSave,
  change,
  onOpen,
  onAsk,
  onAdd,
  onSignIn,
  onBack,
  notices
}: Props) => {
  const [section, setSection] = useState<SettingsSection | 'servers'>('appearance');
  const [drawerOpen, setDrawerOpen] = useState(false);

  return (
    <>
      <header className="bar">
        <button
          type="button"
          className="ghost settings-menu"
          aria-label="Settings sections"
          aria-expanded={drawerOpen}
          onClick={() => setDrawerOpen((open) => !open)}
        >
          ☰
        </button>

        <h1>Settings</h1>

        <button type="button" className="ghost" onClick={onBack}>
          Back
        </button>
      </header>

      <main className="content">
        {notices}

        {drawerOpen ? <div className="settings-scrim" onClick={() => setDrawerOpen(false)} aria-hidden="true" /> : null}

        <nav className={drawerOpen ? 'settings-drawer open' : 'settings-drawer'} aria-label="Settings sections">
          {SECTIONS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={entry.id === section ? 'settings-section active' : 'settings-section'}
              aria-current={entry.id === section}
              onClick={() => {
                setSection(entry.id);
                setDrawerOpen(false);
              }}
            >
              {entry.label}
            </button>
          ))}
        </nav>

        <h2 className="section">{SECTIONS.find((entry) => entry.id === section)?.label}</h2>

        {section === 'appearance' || section === 'about' ? (
          <SettingsScreen settings={registry.settings} onSave={onSave} section={section} />
        ) : null}

        {section === 'notifications' ? (
          <>
            <SettingsScreen settings={registry.settings} onSave={onSave} section="notifications" />

            <h2 className="section">While Shiver is closed</h2>

            <BackgroundNotifications />
          </>
        ) : null}

        {section === 'servers' ? (
          <>
            <ServerList
              servers={servers}
              onOpen={onOpen}
              // asked first, as from the rail's menu
              onRemove={(id) => onAsk('remove', id)}
              onAdd={onAdd}
              signedOut={sessions.signedOut}
              problems={sessions.problems}
              onAcceptAnySize={(id, accept) => void change(() => api.setAcceptAnySize(id, accept))}
              onNotifyLevel={(id, level) => void change(() => api.setNotifyLevel(id, level))}
              remembered={sessions.remembered}
              plugins={sessions.plugins}
              onSignIn={onSignIn}
            />

            <h2 className="section">Background sessions</h2>

            <Sessions onCleared={onSessionsChanged} />

            <h2 className="section">Links</h2>

            <TrustedLinks count={registry.settings.trustedLinks.length} />
          </>
        ) : null}
      </main>
    </>
  );
};
