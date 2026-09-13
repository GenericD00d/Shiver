type Props = {
  onAdd: () => void;
};

export const WelcomePanel = ({ onAdd }: Props) => (
  <div className="panel">
    <h1>Welcome to Shiver</h1>
    <p className="hint">
      Shiver holds all of your Sharkord servers in one window. Add the first one to get started.
    </p>

    <div className="actions">
      <button type="button" className="primary" onClick={onAdd}>
        Add a server
      </button>
    </div>
  </div>
);
