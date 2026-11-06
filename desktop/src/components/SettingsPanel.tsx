import { useCallback, useState } from 'react';

import { api } from '../api';
import { DesktopNotifications, LogFolder, PermissionsSection, ServersSection, VoiceSection } from './DesktopSections';
import { playNotificationSound } from '../sounds';
import {
  BellIcon,
  InfoIcon,
  MicIcon,
  PaletteIcon,
  ServerIcon,
  ShieldIcon
} from '../../../shared/web/components/icons';
import { SettingsLayout, useSettingsDraft } from '../../../shared/web/components/SettingsLayout';
import { AboutSection, AppearanceSection, NotificationsSection } from '../../../shared/web/components/SettingsSections';
import { MAX_SOUND_VOLUME, type Settings } from '../types';

type Props = {
  settings: Settings;
  /** resolves true once saved */
  onSave: (settings: Settings) => Promise<boolean>;
  onClose: () => void;
};

/** The settings sections, grouped by what the user came to change. */
const SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: PaletteIcon },
  { id: 'notifications', label: 'Notifications', icon: BellIcon },
  { id: 'voice', label: 'Voice', icon: MicIcon },
  { id: 'permissions', label: 'Permissions', icon: ShieldIcon },
  { id: 'servers', label: 'Servers', icon: ServerIcon },
  { id: 'about', label: 'About', icon: InfoIcon }
] as const;

type SectionId = (typeof SECTIONS)[number]['id'];

export const SettingsPanel = ({ settings, onSave, onClose }: Props) => {
  const [section, setSection] = useState<SectionId>('appearance');
  const { draft, patch, leave, bar } = useSettingsDraft(settings, onSave);

  // `settings` is not read again while the panel is open, so its count would stay as it was
  const [linksCleared, setLinksCleared] = useState(false);
  const clearLinks = useCallback(() => setLinksCleared(true), []);

  return (
    <SettingsLayout
      title="Shiver Settings"
      entries={SECTIONS}
      selected={section}
      onSelect={setSection}
      onBack={() => leave(onClose)}
      footer={bar}
    >
      {section === 'appearance' ? <AppearanceSection draft={draft} onChange={patch} /> : null}

      {section === 'notifications' ? (
        <NotificationsSection
          draft={draft}
          onChange={patch}
          // the setting is saved in another webview from the one that plays the ping, so it plays here
          onTest={playNotificationSound}
          volumeHint={
            <>
              Moves Shiver's ping and each server's own sounds together. Goes to {MAX_SOUND_VOLUME}%, for a ping that has
              to carry over a call. How loud people are in a call is set per person, in the server's own controls.
            </>
          }
        >
          <DesktopNotifications draft={draft} onChange={patch} />
        </NotificationsSection>
      ) : null}

      {section === 'voice' ? <VoiceSection draft={draft} onChange={patch} /> : null}

      {section === 'permissions' ? (
        <PermissionsSection trustedLinks={linksCleared ? 0 : settings.trustedLinks.length} onLinksForgotten={clearLinks} />
      ) : null}

      {section === 'servers' ? <ServersSection draft={draft} onChange={patch} /> : null}

      {section === 'about' ? (
        <>
          <AboutSection
            appVersion={api.appVersion}
            available={api.availableUpdate}
            checkForUpdate={api.checkForUpdate}
            openRepository={api.openRepository}
            takeLabel="Install Shiver"
            takingNote="Downloading…"
            onTake={api.installUpdate}
          />
          <LogFolder />
        </>
      ) : null}
    </SettingsLayout>
  );
};
