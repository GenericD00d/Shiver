import type { ServerEntry } from '../types';

type Props = {
  server: ServerEntry | undefined;
  onRemove: (id: string) => void;
  onCancel: () => void;
};

/** Asks first: the rail's menu removes a server, and everything Shiver keeps for it, in one click. */
export const RemoveServerPanel = ({ server, onRemove, onCancel }: Props) =>
  server ? (
    <div className="modal-backdrop">
      <div className="card">
        <h1>Remove {server.name}?</h1>
        <p className="hint">
          Shiver forgets its sign-in, saved password, notifications and page storage. Nothing changes on the server.
        </p>

        <div className="actions">
          <button type="button" className="ghost" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="primary" onClick={() => onRemove(server.id)} autoFocus>
            Remove
          </button>
        </div>
      </div>
    </div>
  ) : null;
