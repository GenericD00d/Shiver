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

/** The call's controls in the rail (always visible); they act on the server holding the call. */
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
