import { useCallback, useState } from 'react';

import type { Folder } from '../types';

type Props = {
  folder: Folder | undefined;
  onSave: (id: string, name: string) => void;
  onCancel: () => void;
};

export const RenameFolderPanel = ({ folder, onSave, onCancel }: Props) => {
  const [name, setName] = useState(folder?.name ?? '');

  const handleChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    setName(event.target.value);
  }, []);

  const handleSubmit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();

      if (!folder || !name.trim()) return;

      onSave(folder.id, name.trim());
    },
    [folder, name, onSave]
  );

  if (!folder) return null;

  return (
    <div className="modal-backdrop">
      <form className="card" onSubmit={handleSubmit}>
        <h1>Rename folder</h1>

        <label className="field">
          <span>Folder name</span>
          <input type="text" value={name} onChange={handleChange} autoFocus spellCheck={false} />
        </label>

        <div className="actions">
          <button type="button" className="ghost" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!name.trim()}>
            Save
          </button>
        </div>
      </form>
    </div>
  );
};
