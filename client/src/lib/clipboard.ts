// ── Clipboard helper ──
//
// `navigator.clipboard` only exists in a *secure context*: HTTPS, or localhost.
// Development on http://localhost therefore has it, while a self-hosted
// deployment reached over plain http://<ip>:<port> does not — there the property
// is `undefined` and calling `.writeText()` throws a TypeError. Left unhandled
// inside an async click handler that rejection is invisible, so the button just
// appears dead.
//
// Callers get a boolean instead of a throwing promise, so a failed copy can be
// surfaced to the user (e.g. show the link for manual copying) rather than
// silently swallowed.

/** Copy `text`, returning whether it made it to the clipboard. Never throws. */
export async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Denied permission, or a non-secure context that still exposes the API.
      // Fall through to the legacy path.
    }
  }

  // Legacy `execCommand` path: deprecated, but the only option on plain http.
  // Requires a live selection, hence the off-screen textarea.
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.top = '-9999px';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    ta.setSelectionRange(0, text.length);
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
