/**
 * Inline icons rather than emoji: emoji are rendered by the system font in their own colours (the
 * bell comes out gold), which cannot be themed. These inherit `currentColor`, so they follow Shiver's
 * text colour and the user's accent. Both clients draw these.
 */

type Props = {
  className?: string;
};

const base = {
  width: 16,
  height: 16,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 2,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  'aria-hidden': true
};

export const BellIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M18 8a6 6 0 1 0-12 0c0 7-3 9-3 9h18s-3-2-3-9" />
    <path d="M13.73 21a2 2 0 0 1-3.46 0" />
  </svg>
);

export const MessagesIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
  </svg>
);

export const SettingsIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1" />
  </svg>
);

export const PlusIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M12 5v14M5 12h14" />
  </svg>
);

/*
 * The voice set, drawn to match the lucide icons Sharkord uses for the same controls, so Shiver's
 * copy of a control reads as the same control.
 */

export const MicIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3" />
    <path d="M19 10v2a7 7 0 0 1-14 0v-2M12 19v3" />
  </svg>
);

export const MicOffIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="m2 2 20 20" />
    <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V5a3 3 0 0 0-5.94-.6" />
    <path d="M17 16.95A7 7 0 0 1 5 12v-2M19 10v2a7 7 0 0 1-.11 1.23M12 19v3" />
  </svg>
);

export const HeadphonesIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM18 14h3v5a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2" />
    <path d="M3 14v-2a9 9 0 0 1 18 0v2" />
  </svg>
);

export const HeadphoneOffIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="m2 2 20 20" />
    <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    <path d="M21 14v5a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h1" />
    <path d="M3 14v-2a9 9 0 0 1 13.6-7.75M21 14v-2a8.9 8.9 0 0 0-.6-3.2" />
  </svg>
);

/** Lucide's `phone-off`; its slash crosses the handset's diagonal, unlike the other "off" icons. */
export const PhoneOffIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M10.1 13.9a14 14 0 0 0 3.732 2.668 1 1 0 0 0 1.213-.303l.355-.465A2 2 0 0 1 17 15h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2 18 18 0 0 1-12.728-5.272" />
    <path d="M22 2 2 22" />
    <path d="M4.76 13.582A18 18 0 0 1 2 4a2 2 0 0 1 2-2h3a2 2 0 0 1 2 2v3a2 2 0 0 1-.8 1.6l-.468.351a1 1 0 0 0-.292 1.233 14 14 0 0 0 .244.473" />
  </svg>
);

/* The settings set: the back and menu buttons, and one per section, drawn as Sharkord's own settings draw theirs. */

export const ChevronLeftIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="m15 18-6-6 6-6" />
  </svg>
);

export const MenuIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M4 12h16M4 6h16M4 18h16" />
  </svg>
);

export const PaletteIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <circle cx="13.5" cy="6.5" r=".5" fill="currentColor" />
    <circle cx="17.5" cy="10.5" r=".5" fill="currentColor" />
    <circle cx="8.5" cy="7.5" r=".5" fill="currentColor" />
    <circle cx="6.5" cy="12.5" r=".5" fill="currentColor" />
    <path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.93 0 1.65-.75 1.65-1.69 0-.44-.18-.83-.44-1.12-.29-.29-.44-.65-.44-1.12a1.64 1.64 0 0 1 1.67-1.67h2c3.05 0 5.56-2.5 5.56-5.55C21.97 6.01 17.46 2 12 2z" />
  </svg>
);

export const ShieldIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z" />
  </svg>
);

export const ServerIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <rect width="20" height="8" x="2" y="2" rx="2" ry="2" />
    <rect width="20" height="8" x="2" y="14" rx="2" ry="2" />
    <path d="M6 6h.01M6 18h.01" />
  </svg>
);

export const InfoIcon = ({ className }: Props) => (
  <svg {...base} className={className}>
    <circle cx="12" cy="12" r="10" />
    <path d="M12 16v-4M12 8h.01" />
  </svg>
);
