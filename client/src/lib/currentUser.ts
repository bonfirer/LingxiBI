import type { CurrentUser } from './api';

/** Read the cached current user (populated by App on load) from localStorage. */
export function getCurrentUser(): CurrentUser | null {
  try {
    const raw = localStorage.getItem('user');
    return raw ? (JSON.parse(raw) as CurrentUser) : null;
  } catch {
    return null;
  }
}

/** Whether the current user is an admin. */
export function isAdmin(): boolean {
  return getCurrentUser()?.role === 'admin';
}
