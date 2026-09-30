import { useEffect, useState } from 'react';

import './ui.css';

type Props = {
  /** the server coming up; null while Shiver itself is starting */
  serverName: string | null;
  /** the server has had long enough and has not come up */
  failed?: boolean;
  onRetry?: () => void;
  /** when the core will open it, if it is waiting for the server to take another join */
  until?: number;
};

/** A spinner while a server's client comes up, or the failure with a retry. */
export const Connecting = ({ serverName, failed = false, onRetry, until }: Props) => (
  <div className="connecting">
    {failed ? (
      <>
        <p className="connecting-failed">Failed to connect to {serverName}</p>
        {onRetry ? (
          <button type="button" className="ghost" onClick={onRetry}>
            Try again
          </button>
        ) : null}
      </>
    ) : (
      <>
        <span className="spinner" aria-hidden="true" />
        <p role="status">{serverName ? `Connecting to ${serverName}…` : 'Starting Shiver…'}</p>
        {until ? <JoinWait until={until} /> : null}
      </>
    )}
  </div>
);

/** Counts down a wait for the server to take another join, and says why there is one. */
const JoinWait = ({ until }: { until: number }) => {
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);

    return () => window.clearInterval(timer);
  }, []);

  const seconds = Math.max(0, Math.ceil((until - now) / 1000));

  return (
    <p>
      {seconds > 0 ? `In ${seconds} s: ` : ''}Sharkord lets an account connect only a few times a minute, so Shiver
      waits rather than be turned away.
    </p>
  );
};
