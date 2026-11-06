import { type ReactNode, useState } from 'react';

import { api } from '../api';
import type { ConfirmAction } from './ActionConfirm';
import { BackgroundNotifications } from './BackgroundNotifications';
import { ServerList } from './ServerList';
import { Sessions, TrustedLinks } from './Sessions';
import { BellIcon, InfoIcon, PaletteIcon, ServerIcon } from '../../../shared/web/components/icons';
import { SettingsCard, SettingsLayout, useSettingsDraft } from '../../../shared/web/components/SettingsLayout';
import { AboutSection, AppearanceSection, NotificationsSection } from '../../../shared/web/components/SettingsSections';
import { MAX_SOUND_VOLUME, type Registry, type ServerEntry, type SessionStates, type Settings } from '../types';

/** The settings sections, in the order they are listed. */
const SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: PaletteIcon },
  { id: 'notifications', label: 'Notifications', icon: BellIcon },
  { id: 'servers', label: 'Servers', icon: ServerIcon },
  { id: 'about', label: 'About', icon: InfoIcon }
] as const;

type SectionId = (typeof SECTIONS)[number]['id'];

type Props = {
  registry: Registry;
  /** in rail order, with their logos */
  servers: ServerEntry[];
  sessions: SessionStates;
  /** reads the session states again */
  onSessionsChanged: () => void;
  /** resolves true once saved */
  onSave: (settings: Settings) => Promise<boolean>;
  /** runs a change, then reads everything again */
  change: (run: () => Promise<unknown>) => Promise<unknown>;
  onOpen: (entryId: string) => void;
  onAsk: (action: ConfirmAction, entryId: string) => void;
  onAdd: () => void;
  onSignIn: (entryId: string) => void;
  onBack: () => void;
  /** above the cards: an error, the update notice */
  notices?: ReactNode;
};

/**
 * Shiver's settings, in the layout Sharkord's own settings have (`SettingsLayout`): on a phone the
 * sections are a drawer behind the bar's menu button. Drawn in Shiver's page and over a server's
 * page alike; one draft for every section, saved from the bar that appears once it differs.
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
  const [section, setSection] = useState<SectionId>('appearance');
  const { draft, patch, leave, bar } = useSettingsDraft(registry.settings, onSave);

  return (
    <SettingsLayout
      title="Shiver Settings"
      entries={SECTIONS}
      selected={section}
      onSelect={setSection}
      onBack={() => leave(onBack)}
      notices={notices}
      footer={bar}
    >
      {section === 'appearance' ? <AppearanceSection draft={draft} onChange={patch} /> : null}

      {section === 'notifications' ? (
        <>
          <NotificationsSection
            draft={draft}
            onChange={patch}
            volumeHint={
              <>
                Your servers' own sounds, up to {MAX_SOUND_VOLUME}% so a ping carries over a call. Takes effect next time
                you open a server.
              </>
            }
          />

          <SettingsCard title="While Shiver is closed">
            <BackgroundNotifications />
          </SettingsCard>
        </>
      ) : null}

      {section === 'servers' ? (
        <>
          <SettingsCard title="Servers">
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
          </SettingsCard>

          <SettingsCard title="Background sessions">
            <Sessions onCleared={onSessionsChanged} />
          </SettingsCard>

          <SettingsCard title="Links">
            <TrustedLinks count={registry.settings.trustedLinks.length} />
          </SettingsCard>
        </>
      ) : null}

      {section === 'about' ? (
        <AboutSection
          appVersion={api.appVersion}
          available={api.updateAvailable}
          checkForUpdate={api.checkForUpdate}
          openRepository={api.openRepository}
          takeLabel="Get Shiver"
          takeHint="Android will not let an app install itself, so this opens the download."
          onTake={api.openReleases}
        />
      ) : null}
    </SettingsLayout>
  );
};
