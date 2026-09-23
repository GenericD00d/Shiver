type Props = {
  serverName: string;
  /** the server has had long enough and has not come up */
  failed: boolean;
  onRetry: () => void;
};

/** A spinner while a server's client comes up, or the failure with a retry (the page is covered meanwhile). */
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
