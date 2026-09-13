type Props = {
  serverName: string;
  /** the server has had long enough and has not come up */
  failed: boolean;
  onRetry: () => void;
};

/**
 * What Shiver shows while a server's client is coming up, and when it does not.
 *
 * Deliberately just a spinner. Shiver used to draw the server's last known messages here from a local
 * cache, rebuilt to Sharkord's own measurements — and it was not worth it: what it could show was
 * always a partial copy of the client, it only covered channels already visited, and reproducing
 * the client's own view is the one thing Shiver exists not to do.
 *
 * The failure state matters as much as the wait. Shiver covers the page while it waits, so a wait
 * with no end is a page the user cannot reach; saying so, with a way to try again, is the way out.
 */
export const ConnectingPanel = ({ serverName, failed, onRetry }: Props) => (
  <div className="connecting">
    {failed ? (
      <>
        <p className="connecting-failed">Failed to connect to {serverName}</p>
        <button type="button" className="ghost" onClick={onRetry}>
          Try again
        </button>
      </>
    ) : (
      <>
        <span className="spinner" aria-hidden="true" />
        <p>Connecting to {serverName}…</p>
      </>
    )}
  </div>
);
