import type { ApiTokenCreatedResponse } from "@/api/generated/ApiTokenCreatedResponse";
import type { ApiTokenResponse } from "@/api/generated/ApiTokenResponse";
import type { ArtifactResponse } from "@/api/generated/ArtifactResponse";
import type { CreateApiTokenRequest } from "@/api/generated/CreateApiTokenRequest";
import type { CreateRepositoryRequest } from "@/api/generated/CreateRepositoryRequest";
import type { CreateRepositoryResponse } from "@/api/generated/CreateRepositoryResponse";
import type { ErrorResponse } from "@/api/generated/ErrorResponse";
import type { LoginRequest } from "@/api/generated/LoginRequest";
import type { LoginResponse } from "@/api/generated/LoginResponse";
import type { PublishResponse } from "@/api/generated/PublishResponse";
import type { RepositoryResponse } from "@/api/generated/RepositoryResponse";
import type { UserResponse } from "@/api/generated/UserResponse";

/**
 * Todas las peticiones se dirigen a `/api`, que el servidor de
 * desarrollo de Vite reenvía a `http://127.0.0.1:3000` (ver
 * `vite.config.ts`), evitando problemas de CORS sin necesidad de
 * codificar ninguna URL absoluta en el cliente.
 */
const API_BASE_URL = "/api";

const TOKEN_STORAGE_KEY = "ferrobox.auth.token";

/** Error tipado lanzado por el cliente HTTP ante cualquier respuesta no exitosa. */
export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

export function getStoredToken(): string | null {
  return localStorage.getItem(TOKEN_STORAGE_KEY);
}

export function setStoredToken(token: string | null): void {
  if (token === null) {
    localStorage.removeItem(TOKEN_STORAGE_KEY);
  } else {
    localStorage.setItem(TOKEN_STORAGE_KEY, token);
  }
}

async function extractErrorMessage(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as ErrorResponse;
    if (typeof body.error === "string" && body.error.length > 0) {
      return body.error;
    }
  } catch {
    // El cuerpo no era JSON (o estaba vacío) -- se usa el mensaje genérico.
  }
  return `${response.status} ${response.statusText}`;
}

function authHeaders(extra?: HeadersInit): Headers {
  const headers = new Headers(extra);
  const token = getStoredToken();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  return headers;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = authHeaders({
    "content-type": "application/json",
    ...init?.headers,
  });

  const response = await fetch(`${API_BASE_URL}${path}`, {
    ...init,
    headers,
  });

  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

export function login(payload: LoginRequest): Promise<LoginResponse> {
  return request<LoginResponse>("/auth/login", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getMe(): Promise<UserResponse> {
  return request<UserResponse>("/auth/me");
}

export function listApiTokens(): Promise<ApiTokenResponse[]> {
  return request<ApiTokenResponse[]>("/auth/tokens");
}

export function createApiToken(
  payload: CreateApiTokenRequest,
): Promise<ApiTokenCreatedResponse> {
  return request<ApiTokenCreatedResponse>("/auth/tokens", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function revokeApiToken(tokenId: string): Promise<void> {
  return request<void>(`/auth/tokens/${tokenId}`, { method: "DELETE" });
}

export function listRepositories(): Promise<RepositoryResponse[]> {
  return request<RepositoryResponse[]>("/repositories");
}

export function getRepository(repositoryId: string): Promise<RepositoryResponse> {
  return request<RepositoryResponse>(`/repositories/${repositoryId}`);
}

export function createRepository(
  payload: CreateRepositoryRequest,
): Promise<CreateRepositoryResponse> {
  return request<CreateRepositoryResponse>("/repositories", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listRepositoryArtifacts(repositoryId: string): Promise<ArtifactResponse[]> {
  return request<ArtifactResponse[]>(`/repositories/${repositoryId}/artifacts`);
}

export async function publishArtifact(
  repositoryId: string,
  file: File,
): Promise<PublishResponse> {
  const response = await fetch(`${API_BASE_URL}/repositories/${repositoryId}/artifacts`, {
    method: "POST",
    headers: authHeaders({ "content-type": "application/octet-stream" }),
    body: file,
  });

  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }

  return (await response.json()) as PublishResponse;
}

/** Descarga un artefacto con autenticación y dispara el guardado local. */
export async function downloadArtifact(artifactId: string, filename?: string): Promise<void> {
  const response = await fetch(`${API_BASE_URL}/artifacts/${artifactId}`, {
    headers: authHeaders(),
  });

  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }

  const blob = await response.blob();
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = filename ?? artifactId;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(objectUrl);
}

/**
 * Respuesta de `GET /cargo/{repository_id}/config.json`. No se genera
 * con `ts-rs` porque es un detalle del protocolo de Cargo, no un DTO de
 * la API de gestión que consume este frontend -- pero su forma es
 * estable y pública (la define el propio protocolo de registro de
 * Cargo), así que tipar la respuesta a mano es seguro.
 */
export interface CargoRegistryConfig {
  dl: string;
  api: string;
}

export function getCargoRegistryConfig(repositoryId: string): Promise<CargoRegistryConfig> {
  return request<CargoRegistryConfig>(`/cargo/${repositoryId}/config.json`);
}
