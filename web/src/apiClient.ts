export class ApiError extends Error {
  constructor(message: string, readonly status: number, readonly code?: string) {
    super(message);
    this.name = "ApiError";
  }
}

export async function apiRequest<T>(apiBase: string, path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiBase + path, {
    ...init, credentials: "include",
    headers: { "content-type": "application/json", ...init?.headers }
  });
  if (!response.ok) {
    const body: unknown = await response.json().catch(() => null);
    const detail = body && typeof body === "object" ? body as Record<string, unknown> : {};
    throw new ApiError(
      typeof detail.error === "string" ? detail.error : response.statusText || "Request failed (HTTP " + response.status + ").",
      response.status, typeof detail.code === "string" ? detail.code : undefined
    );
  }
  return response.json() as Promise<T>;
}