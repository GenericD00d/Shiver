import { useCallback, useState } from 'react';

import { api } from '../api';
import { AboutSection, AppearanceSection, NotificationsSection } from '../../../shared/web/components/SettingsSections';
import { MAX_SOUND_VOLUME, type Settings } from '../types';

/** Which group of settings to draw. The rest live in their own components — see `App`. */
export type SettingsSection = 'appearance' | 'notifications' | 'about';

type Props = {
  settings: Settings;
  onSave: (settings: Settings) => void;
  section: SettingsSection;
};

/** One of the sections both clients share (the sections are listed by `App`). */
export const SettingsScreen = ({ settings, onSave, section }: Props) => {
  const [draft, setDraft] = useState<Settings>(settings);

  const patch = useCallback((changes: Partial<Settings>) => {
    setDraft((current) => ({ ...current, ...changes }));
  }, []);

  if (section === 'about') {
    return (
      <AboutSection
        appVersion={api.appVersion}
        available={api.updateAvailable}
        checkForUpdate={api.checkForUpdate}
        openRepository={api.openRepository}
        takeLabel="Get Shiver"
        takeHint="Android will not let an app install itself, so this opens the download."
        onTake={api.openReleases}
      />
    );
  }

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        onSave(draft);
      }}
    >
      {section === 'appearance' ? <AppearanceSection draft={draft} onChange={patch} /> : null}

      {section === 'notifications' ? (
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
      ) : null}

      <div className="actions">
        <button type="submit" className="primary">
          Save
        </button>
      </div>
    </form>
  );
};
