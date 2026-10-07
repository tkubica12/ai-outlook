import mascot from "../assets/generated/duckpilot-mascot.png";

export function DuckPilotLogo({ compact = false }: { compact?: boolean }) {
  return (
    <img
      className={compact ? "duckpilot-logo compact" : "duckpilot-logo"}
      src={mascot}
      alt="Tomlook mascot, a yellow rubber duck wearing aviator goggles"
    />
  );
}
