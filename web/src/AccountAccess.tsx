import { useEffect, useRef, useState, type FormEvent } from "react";
import { apiRequest, clearGuest, ensureGuest, type GuestSession } from "./apiClient";

export function AccountAccess({ apiBase, playerId, onAuthenticated, onSignOut }: {
  apiBase: string; playerId: string; onAuthenticated: (session: GuestSession) => void; onSignOut: () => Promise<void>;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [session, setSession] = useState<GuestSession | null>(null);
  const [mode, setMode] = useState<"login" | "register" | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    setMode(null); setPassword("");
    let active = true;
    if (playerId) void ensureGuest(apiBase).then(value => { if (active) setSession(value); }).catch(() => {});
    else setSession(null);
    return () => { active = false; };
  }, [apiBase, playerId]);
  useEffect(() => { if (mode && dialog.current && !dialog.current.open) dialog.current.showModal(); }, [mode]);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!mode || pending) return;
    setPending(true); setError("");
    try {
      await apiRequest<GuestSession>(apiBase, `/v1/auth/${mode}`, {
        method: "POST", body: JSON.stringify({ username, password }),
      });
      clearGuest(true);
      const current = await ensureGuest(apiBase);
      setSession(current); setPassword(""); setMode(null); onAuthenticated(current);
    } catch (failure) { setError((failure as Error).message); }
    finally { setPending(false); }
  }
  if (!session) return null;
  const signOut = <button className="secondary" onClick={() => void onSignOut()}>{session.username ? "Sign out" : "End guest session"}</button>;
  if (!session.accounts_available) return signOut;
  if (session.username) return <><span className="muted">Signed in as {session.username}</span>{signOut}</>;
  return <>
    {signOut}
    <button className="secondary" onClick={() => { setMode("login"); setError(""); }}>Sign in</button>
    <button className="secondary" onClick={() => { setMode("register"); setError(""); }}>Save guest as account</button>
    {mode && <dialog ref={dialog} className="account-dialog" aria-labelledby="account-title" onCancel={event => { if (pending) event.preventDefault(); else { setMode(null); setPassword(""); } }}>
      <form onSubmit={event => void submit(event)}>
        <h2 id="account-title">{mode === "login" ? "Sign in" : "Save your account"}</h2>
        <p>{mode === "register" ? "Keep your current player identity, games, and role access when you sign in again." : "Return to your saved player identity."}</p>
        <label>Username<input autoFocus autoComplete="username" value={username} minLength={3} maxLength={40} pattern="[A-Za-z0-9_]{3,40}" required onChange={event => setUsername(event.target.value)} /></label>
        <label>Password<input type="password" autoComplete={mode === "register" ? "new-password" : "current-password"} value={password} minLength={12} maxLength={128} required onChange={event => setPassword(event.target.value)} /></label>
        {mode === "register" && <small>Use 12–128 characters. Username accepts letters, numbers, and underscores.</small>}
        {error && <p role="alert">{error}</p>}
        <div className="account-actions">
          <button type="button" className="secondary" disabled={pending} onClick={() => { setMode(null); setPassword(""); }}>Cancel</button>
          <button type="submit" disabled={pending}>{pending ? (mode === "register" ? "Saving…" : "Signing in…") : mode === "register" ? "Create account" : "Sign in"}</button>
        </div>
      </form>
    </dialog>}
  </>;
}
