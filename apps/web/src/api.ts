export class ApiError extends Error {
  constructor(public status: number, message: string, public code = 'request_failed') {
    super(message)
  }
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const hasJsonBody = init.body !== undefined && !(init.body instanceof FormData)
  const response = await fetch(path, {
    credentials: 'include',
    cache: 'no-store',
    ...init,
    headers: {
      ...(hasJsonBody ? { 'content-type': 'application/json' } : {}),
      ...init.headers,
    },
  })
  if (!response.ok) {
    const payload = await response.json().catch(() => null)
    throw new ApiError(
      response.status,
      payload?.error?.message || `Request failed with ${response.status}`,
      payload?.error?.code,
    )
  }
  if (response.status === 204) return undefined as T
  return response.json()
}

export function post<T>(path: string, body?: unknown) {
  return api<T>(path, { method: 'POST', body: body === undefined ? undefined : JSON.stringify(body) })
}

export function put<T>(path: string, body: unknown) {
  return api<T>(path, { method: 'PUT', body: JSON.stringify(body) })
}

export function patch<T>(path: string, body: unknown) {
  return api<T>(path, { method: 'PATCH', body: JSON.stringify(body) })
}
