// localStorage wrapped in try/catch: private windows, storage-blocked
// browsers, and non-DOM test environments must all degrade to "no
// persistence" rather than throwing.

export function storageGet(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function storageSet(key: string, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // best effort
  }
}

export function storageDel(key: string): void {
  try {
    globalThis.localStorage?.removeItem(key);
  } catch {
    // best effort
  }
}

/** JSON convenience over storageGet (malformed → null). */
export function storageGetJson<T>(key: string): T | null {
  const raw = storageGet(key);
  if (raw === null) {
    return null;
  }
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

export function storageSetJson(key: string, value: unknown): void {
  try {
    storageSet(key, JSON.stringify(value));
  } catch {
    // best effort
  }
}
