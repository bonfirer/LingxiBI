import { useState } from 'react';
import { useParams } from 'react-router-dom';

/**
 * Shared report page — renders the server-generated H5 page for a share token.
 *
 * The report HTML is LLM-authored and therefore untrusted, so it is embedded in
 * a sandboxed iframe rather than navigated to at the top level. Navigating made
 * the report the app-origin document with no sandbox at all, which let a report
 * read this origin's localStorage and act as any visitor who happened to be
 * logged in. `allow-scripts` alone gives it an opaque origin: charts still run,
 * but it has no access to app-origin storage or credentials. The server also
 * sends a matching `sandbox` CSP as a second layer.
 */
export default function SharedReportPage() {
  const { token } = useParams<{ token: string }>();
  const [loading, setLoading] = useState(true);

  if (!token) return null;

  return (
    <div className="min-h-screen bg-obsidian-950">
      {loading && (
        <div className="fixed inset-0 flex items-center justify-center">
          <div className="w-5 h-5 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
        </div>
      )}
      <iframe
        src={`/api/share/${encodeURIComponent(token)}/html`}
        className="w-screen h-screen border-0 block"
        sandbox="allow-scripts allow-popups allow-forms allow-modals"
        title="Shared report"
        onLoad={() => setLoading(false)}
      />
    </div>
  );
}
