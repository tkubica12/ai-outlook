import { CheckCircle2, CircleAlert, Server, X } from "lucide-react";
import { useCallback, useEffect, useId, useState } from "react";
import { api } from "../api";
import { useDialog } from "../useDialog";

type DiagnosticData = Awaited<ReturnType<typeof api.diagnostics>>;

export function Diagnostics({ onClose }: { onClose: () => void }) {
  const [data, setData] = useState<DiagnosticData | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const dialogRef = useDialog<HTMLDivElement>(onClose);
  const titleId = useId();

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      setData(await api.diagnostics());
    } catch (reason) {
      setData(null);
      setError(reason instanceof Error ? reason.message : "Diagnostics unavailable");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => void load(), [load]);

  return (
    <div className="modal-layer diagnostic-layer">
      <div
        ref={dialogRef}
        className="diagnostics-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="modal-title">
          <div>
            <span className="proposal-label">Developer tools</span>
            <h2 id={titleId}>System diagnostics</h2>
            <p>Runtime, connector capabilities, and data provenance.</p>
          </div>
          <button className="icon-button" onClick={onClose} aria-label="Close diagnostics">
            <X size={19} />
          </button>
        </div>

        {error && (
          <div className="error-banner" role="alert">
            <span>{error}</span>
            <button onClick={load}>Retry</button>
          </div>
        )}
        {loading && (
          <div className="diagnostic-loading" role="status" aria-busy="true">
            Checking local services…
          </div>
        )}
        {data && (
          <>
            <div className="health-cards">
              <div>
                <Server size={18} aria-hidden="true" />
                <span>
                  <small>Backend</small>
                  <strong>{data[0].status}</strong>
                </span>
                <CheckCircle2 size={16} aria-hidden="true" />
              </div>
              <div>
                <span className="source-icon" aria-hidden="true">
                  AI
                </span>
                <span>
                  <small>Agent runtime</small>
                  <strong>{data[0].runtime}</strong>
                </span>
              </div>
              <div>
                <span className="source-icon" aria-hidden="true">
                  DB
                </span>
                <span>
                  <small>Storage</small>
                  <strong>{data[0].database}</strong>
                </span>
              </div>
            </div>
            <h3>Connector capabilities</h3>
            <div className="connector-list">
              {data[1].map((connector) => (
                <div key={connector.id}>
                  <span className={`status-dot ${connector.status}`} aria-hidden="true" />
                  <span>
                    <strong>{connector.label}</strong>
                    <small>{connector.capabilities.join(" · ")}</small>
                  </span>
                  <span className="mode-chip">{connector.mode}</span>
                  <span className="visually-hidden">{connector.status}</span>
                  {connector.status === "degraded" ? (
                    <CircleAlert size={15} aria-hidden="true" />
                  ) : (
                    <CheckCircle2 size={15} aria-hidden="true" />
                  )}
                </div>
              ))}
            </div>
            <div className="diagnostic-note">
              <CircleAlert size={17} aria-hidden="true" />
              <p>
                <strong>Live MCP mode.</strong> Each connector is discovered independently. Missing
                or denied sources remain unavailable without replacing them with synthetic data;
                credentials never reach the browser.
              </p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
