import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';

import type { MenuEntry } from '../menus';
import './menu.css';

type Props = {
  entries: MenuEntry[];
  /** where it opens: beside the rail, level with the tile it is for */
  at: { x: number; y: number };
  onChoose: (id: string) => void;
  onClose: () => void;
};

/** How far from the viewport's edges the menu keeps. */
const MARGIN = 8;

/**
 * A rail menu drawn in the page (the desktop client shows the same items as a native menu). A
 * submenu opens in place, under its label. Anything outside closes it, on the press rather than the
 * click: the click that ends the press that opened it lands outside too.
 */
export const Menu = ({ entries, at, onChoose, onClose }: Props) => {
  const node = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: at.x, top: at.y });
  const [open, setOpen] = useState<string | null>(null);

  // kept on screen: measured once drawn, then moved back inside if it runs off an edge
  useLayoutEffect(() => {
    const box = node.current?.getBoundingClientRect();

    if (!box) return;

    setPlace({
      left: Math.max(MARGIN, Math.min(at.x, window.innerWidth - box.width - MARGIN)),
      top: Math.max(MARGIN, Math.min(at.y, window.innerHeight - box.height - MARGIN))
    });
  }, [at.x, at.y, open]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    const onPress = (event: PointerEvent) => {
      if (!node.current?.contains(event.target as Node)) onClose();
    };

    window.addEventListener('keydown', onKey);
    window.addEventListener('pointerdown', onPress, true);

    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('pointerdown', onPress, true);
    };
  }, [onClose]);

  const draw = (list: MenuEntry[]) =>
    list.map((entry, index) => {
      if (entry.kind === 'separator') return <hr key={`separator-${index}`} />;

      if (entry.kind === 'submenu') {
        const expanded = open === entry.label;

        return (
          <Fragment key={entry.label}>
            <button
              type="button"
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={expanded}
              className="menu-submenu"
              onClick={() => setOpen(expanded ? null : entry.label)}
            >
              {entry.label}
              <span aria-hidden="true">{expanded ? '▾' : '▸'}</span>
            </button>

            {expanded ? (
              <div className="menu-nested" role="group" aria-label={entry.label}>
                {draw(entry.items)}
              </div>
            ) : null}
          </Fragment>
        );
      }

      return (
        <button
          key={entry.id}
          type="button"
          role={entry.checked === undefined ? 'menuitem' : 'menuitemradio'}
          aria-checked={entry.checked}
          disabled={entry.disabled}
          onClick={() => {
            onClose();
            onChoose(entry.id);
          }}
        >
          {entry.checked !== undefined ? <span className="menu-check">{entry.checked ? '✓' : ''}</span> : null}
          {entry.label}
        </button>
      );
    });

  return (
    <div ref={node} className="menu" role="menu" style={place}>
      {draw(entries)}
    </div>
  );
};
