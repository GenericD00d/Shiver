import { useState } from 'react';

import './ui.css';

type Props = {
  name: string;
  onSave: (name: string) => void;
  onCancel: () => void;
};

/** Renames a rail folder, as a dialog card. */
export const RenameFolderForm = ({ name: current, onSave, onCancel }: Props) => {
  const [name, setName] = useState(current);

  return (
    <form
      className="card"
      onSubmit={(event) => {
        event.preventDefault();

        if (name.trim()) onSave(name.trim());
      }}
    >
      <h1>Rename folder</h1>

      <label className="field">
        <span>Folder name</span>
        <input type="text" value={name} onChange={(event) => setName(event.target.value)} autoFocus spellCheck={false} />
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
  );
};
