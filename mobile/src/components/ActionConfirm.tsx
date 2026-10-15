import { Confirm } from '../../../shared/web/components/Confirm';

/** A rail menu action that is confirmed before it runs. */
export type ConfirmAction = 'remove' | 'forgetpw' | 'logout';

const QUESTIONS: Record<ConfirmAction, { question: (name: string) => string; hint?: string; action: string }> = {
  remove: {
    question: (name) => `Remove ${name}?`,
    hint: 'Shiver forgets its sign-in, saved password and page storage. Nothing changes on the server.',
    action: 'Remove'
  },
  forgetpw: { question: (name) => `Forget the saved password for ${name}?`, action: 'Forget' },
  logout: { question: (name) => `Log out of ${name}?`, action: 'Log out' }
};

/** The menu item that asked sits where the confirming button now is: a touch must not answer both. */
const ARM_MS = 1000;

type Props = {
  action: ConfirmAction;
  serverName: string;
  onAnswer: (confirmed: boolean) => void;
};

/** The question a rail menu action asks first, over whatever screen it was chosen on. */
export const ActionConfirm = ({ action, serverName, onAnswer }: Props) => {
  const { question, hint, action: label } = QUESTIONS[action];

  return (
    <div className="modal-backdrop">
      <Confirm
        question={question(serverName)}
        hint={hint}
        action={label}
        armMs={ARM_MS}
        onConfirm={() => onAnswer(true)}
        onCancel={() => onAnswer(false)}
      />
    </div>
  );
};
