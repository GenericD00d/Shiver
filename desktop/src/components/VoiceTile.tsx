import { useCallback } from 'react';

import { api } from '../api';
import {
  HeadphoneOffIcon,
  HeadphonesIcon,
  MicIcon,
  MicOffIcon,
  PhoneOffIcon
} from './icons';
import type { VoiceStatus } from '../types';

type Props = {
  status: VoiceStatus;
  onOpenServer: (entryId: string) => void;
};

/**
 * The call, in Shiver's own chrome.
 *
 * Voice controls that are there whichever server is on screen, and the rail is
 * the only part of Shiver that is always visible: server webviews start to the right of it, so a
 * control drawn here is never covered by the server the user has navigated to. That is also why it
 * is not a floating overlay like the bell — the bell has to sit over a server's top bar, this does
 * not, so it needs no webview of its own.
 *
 * Every button acts on the server holding the call rather than the one being viewed, which is the
 * whole point of it: the core sends the click to that entry's page.
 */
export const VoiceTile = ({ status, onOpenServer }: Props) => {
  const { channelName, serverName, accountLabel, micMuted, soundMuted, micLocked } = status;

  const control = useCallback(async (action: 'mic' | 'sound' | 'leave') => {
    await api.voiceControl(action).catch(() => undefined);
  }, []);

  return (
    <div className="voice-tile">
      <button
        type="button"
        className="voice-where"
        // the call is somewhere; this is how the user gets back to it
        title={`${channelName ?? 'Voice'} · ${serverName}${accountLabel ? ` (${accountLabel})` : ''}`}
        onClick={() => onOpenServer(status.entryId)}
      >
        <span className="voice-channel">{channelName ?? 'Voice'}</span>
        <span className="voice-server">{serverName}</span>
      </button>

      <div className="voice-buttons">
        <button
          type="button"
          className={micMuted ? 'voice-button muted' : 'voice-button'}
          // Sharkord disables its own mic control while deafened, and disagreeing with it would
          // mean offering a button that quietly does nothing
          disabled={micLocked}
          title={micLocked ? 'Undeafen first' : micMuted ? 'Unmute' : 'Mute'}
          onClick={() => control('mic')}
        >
          {micMuted ? <MicOffIcon /> : <MicIcon />}
        </button>

        <button
          type="button"
          className={soundMuted ? 'voice-button muted' : 'voice-button'}
          title={soundMuted ? 'Undeafen' : 'Deafen'}
          onClick={() => control('sound')}
        >
          {soundMuted ? <HeadphoneOffIcon /> : <HeadphonesIcon />}
        </button>

        <button
          type="button"
          className="voice-button leave"
          title="Disconnect"
          onClick={() => control('leave')}
        >
          <PhoneOffIcon />
        </button>
      </div>
    </div>
  );
};
