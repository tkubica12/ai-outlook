import { CheckCircle2, KeyRound, PlugZap, RotateCcw, ShieldCheck } from "lucide-react";

interface Props {
  missing: string[];
  configured: string[];
  onRetry: () => void;
}

export function ConnectionSetup({ missing, configured, onRetry }: Props) {
  return (
    <section className="connection-setup" aria-labelledby="connection-title">
      <span className="connection-icon"><PlugZap size={28} /></span>
      <p className="connection-kicker">Live data only</p>
      <h2 id="connection-title">Connect your Microsoft 365 workspace</h2>
      <p className="connection-lead">
        Tomlook no longer generates demonstration records. Configure delegated authentication
        for the MCP servers and your actual calendar will appear here.
      </p>
      <div className="connection-steps">
        <div><span>1</span><div><strong>Create your local environment file</strong><code>Copy-Item .env.example .env</code></div></div>
        <div><span>2</span><div><strong>Add short-lived delegated access tokens</strong><p>Work IQ, Fabric, and Dataverse must authorize your signed-in identity.</p></div></div>
        <div><span>3</span><div><strong>Add a newly rotated WebIQ key</strong><p>Do not reuse the key previously pasted into chat.</p></div></div>
        <div><span>4</span><div><strong>Restart the backend</strong><code>.\scripts\start.ps1</code></div></div>
      </div>
      <div className="connection-security">
        <ShieldCheck size={18} />
        <p>Credentials stay in the backend process and are never sent to the browser or stored in SQLite.</p>
      </div>
      {configured.length > 0 && (
        <div className="configured-list">
          <strong>Configured</strong>
          {configured.map((item) => <span key={item}><CheckCircle2 size={14} />{item}</span>)}
        </div>
      )}
      <details>
        <summary><KeyRound size={15} /> Missing environment values ({missing.length})</summary>
        <div className="missing-values">{missing.map((item) => <code key={item}>{item}</code>)}</div>
      </details>
      <button className="primary-action connection-retry" onClick={onRetry}>
        <RotateCcw size={16} /> Check connection again
      </button>
    </section>
  );
}
