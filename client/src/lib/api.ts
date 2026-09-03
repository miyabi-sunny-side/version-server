export interface Asset {
  name: string;
  url: string;
  digest: string | null;
}

// One repo's current release, as `GET /v1/versions` answers it.
export interface Release {
  repo: string;
  tag: string;
  published_at: string | null;
  assets: Asset[];
  source: "webhook" | "poll" | string;
  received_at: string;
}

async function requestJson<T>(url: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(url, { signal });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return (await response.json()) as T;
}

export function fetchVersions(signal?: AbortSignal): Promise<Release[]> {
  return requestJson("/v1/versions", signal);
}
