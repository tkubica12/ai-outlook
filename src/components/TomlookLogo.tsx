export function TomlookLogo({ compact = false }: { compact?: boolean }) {
  return (
    <svg
      className={compact ? "tomlook-logo compact" : "tomlook-logo"}
      viewBox="0 0 64 64"
      role="img"
      aria-label="Tomlook Focus logo"
    >
      <path
        d="M22 10H10V22M42 10H54V22M10 42V54H22M54 42V54H42"
        fill="none"
        stroke="currentColor"
        strokeWidth="6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M22 24H42M32 24V44"
        fill="none"
        stroke="currentColor"
        strokeWidth="6"
        strokeLinecap="square"
      />
    </svg>
  );
}
