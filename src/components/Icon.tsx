const PATHS = {
  logo: 'M12 3l8 4.5v9L12 21l-8-4.5v-9z M12 8v8 M8.5 10l3.5 2 3.5-2',
  folder: 'M3 7h6l2 2h10v10H3z',
  chevronDown: 'M6 9l6 6 6-6',
  chevronRight: 'M9 6l6 6-6 6',
  search: 'M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14z M20 20l-3.5-3.5',
  file: 'M14 3H6v18h12V7z M14 3v4h4',
  git: 'M6 3.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z M6 15.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z M18 6.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z M6 8.5v7 M18 11.5c0 3-4 3.5-10 5',
  history: 'M3 12a9 9 0 1 0 3-6.7 M3 4v5h5 M12 8v4l3 2',
  book: 'M4 5h11a3 3 0 0 1 3 3v11H7a3 3 0 0 1-3-3z M8 9h6 M8 13h6',
  settings: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z',
  terminal: 'M4 17l6-6-6-6 M12 19h8',
  eye: 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z',
  check: 'M5 12l5 5 9-10',
  error: 'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 7v6 M12 16.5v.5',
  warning: 'M12 3l9 16H3z M12 9v4 M12 16v.5',
  info: 'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 11v6 M12 7.5v.5',
  send: 'M5 12h14 M13 6l6 6-6 6',
  stop: 'M7 7h10v10H7z',
  attach: 'M21 12l-8.5 8.5a5 5 0 0 1-7-7L14 5a3.5 3.5 0 0 1 5 5l-8.5 8.5a2 2 0 0 1-3-3L15 8',
  close: 'M6 6l12 12 M18 6L6 18',
  plus: 'M12 5v14 M5 12h14',
  refresh: 'M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7',
  external: 'M14 4h6v6 M20 4l-9 9 M18 14v6H4V6h6',
  edit: 'M4 20h4L19 9l-4-4L4 16z M13.5 6.5l4 4',
  play: 'M7 5l12 7-12 7z',
  diff: 'M12 3v18 M3 12h18',
  undo: 'M9 14L4 9l5-5 M4 9h10a6 6 0 0 1 0 12h-3',
  spark: 'M12 3v4 M12 17v4 M3 12h4 M17 12h4 M6 6l2.5 2.5 M15.5 15.5L18 18 M6 18l2.5-2.5 M15.5 8.5L18 6',
  user: 'M12 4a4 4 0 1 0 0 8a4 4 0 1 0 0-8z M4 21a8 8 0 0 1 16 0',
  cpu: 'M7 7h10v10H7z M9 3v4 M15 3v4 M9 17v4 M15 17v4 M3 9h4 M3 15h4 M17 9h4 M17 15h4',
  trash: 'M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13',
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 16, className }: { name: IconName; size?: number; className?: string }) {
  return (
    <svg className={className ? `icon ${className}` : 'icon'} width={size} height={size} viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}
