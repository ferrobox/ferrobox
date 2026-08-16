import type { ApiTokenCreatedResponse } from "@/api/generated/ApiTokenCreatedResponse";
import type { ApiTokenResponse } from "@/api/generated/ApiTokenResponse";
import type { ArtifactResponse } from "@/api/generated/ArtifactResponse";
import type { AssayLookupRequest } from "@/api/generated/AssayLookupRequest";
import type { AssayResponse } from "@/api/generated/AssayResponse";
import type { AssayRerunResponse } from "@/api/generated/AssayRerunResponse";
import type { ChangePasswordRequest } from "@/api/generated/ChangePasswordRequest";
import type { CreateApiTokenRequest } from "@/api/generated/CreateApiTokenRequest";
import type { CreateRepositoryRequest } from "@/api/generated/CreateRepositoryRequest";
import type { CreateRepositoryResponse } from "@/api/generated/CreateRepositoryResponse";
import type { CreateUserRequest } from "@/api/generated/CreateUserRequest";
import type { ErrorResponse } from "@/api/generated/ErrorResponse";
import type { LoginRequest } from "@/api/generated/LoginRequest";
import type { LoginResponse } from "@/api/generated/LoginResponse";
import type { PublishResponse } from "@/api/generated/PublishResponse";
import type { RepositoryResponse } from "@/api/generated/RepositoryResponse";
import type { RetentionPolicyRequest } from "@/api/generated/RetentionPolicyRequest";
import type { RetentionPolicyResponse } from "@/api/generated/RetentionPolicyResponse";
import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import type { CleanupReportResponse } from "@/api/generated/CleanupReportResponse";
import type { SettingsResponse } from "@/api/generated/SettingsResponse";
import type { UpdateAlloyMembersRequest } from "@/api/generated/UpdateAlloyMembersRequest";
import type { UpdateUserRoleRequest } from "@/api/generated/UpdateUserRoleRequest";
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

export function changePassword(payload: ChangePasswordRequest): Promise<void> {
  return request<void>("/auth/password", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getSettings(): Promise<SettingsResponse> {
  return request<SettingsResponse>("/settings");
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

export function listUsers(): Promise<UserResponse[]> {
  return request<UserResponse[]>("/users");
}

export function createUser(payload: CreateUserRequest): Promise<UserResponse> {
  return request<UserResponse>("/users", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function deleteUser(userId: string): Promise<void> {
  return request<void>(`/users/${userId}`, { method: "DELETE" });
}

export function updateUserRole(
  userId: string,
  payload: UpdateUserRoleRequest,
): Promise<UserResponse> {
  return request<UserResponse>(`/users/${userId}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
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

export function deleteRepository(repositoryId: string): Promise<void> {
  return request<void>(`/repositories/${repositoryId}`, { method: "DELETE" });
}

export function updateAlloyMembers(
  repositoryId: string,
  payload: UpdateAlloyMembersRequest,
): Promise<RepositoryResponse> {
  return request<RepositoryResponse>(`/repositories/${repositoryId}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
}

export function listRepositoryArtifacts(repositoryId: string): Promise<ArtifactResponse[]> {
  return request<ArtifactResponse[]>(`/repositories/${repositoryId}/artifacts`);
}

export function deleteArtifact(repositoryId: string, artifactId: string): Promise<void> {
  return request<void>(`/repositories/${repositoryId}/artifacts/${artifactId}`, {
    method: "DELETE",
  });
}

export function yankCrate(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/cargo/${repositoryId}/api/v1/crates/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankCrate(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/cargo/${repositoryId}/api/v1/crates/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

function npmPackagePath(name: string): string {
  return name.replaceAll("@", "%40").replaceAll("/", "%2F");
}

export function yankNpm(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/npm/${repositoryId}/${npmPackagePath(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankNpm(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/npm/${repositoryId}/${npmPackagePath(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

export function yankPypi(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/pypi/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankPypi(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/pypi/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

export function yankOci(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/oci/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankOci(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/oci/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

export function yankConan(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/conan/${repositoryId}/recipes/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankConan(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/conan/${repositoryId}/recipes/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
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
  "auth-required"?: boolean;
}

export function getCargoRegistryConfig(repositoryId: string): Promise<CargoRegistryConfig> {
  return request<CargoRegistryConfig>(`/cargo/${repositoryId}/config.json`);
}

export function listAssays(): Promise<AssayResponse[]> {
  return request<AssayResponse[]>("/assays");
}

export function listRepositoryAssays(repositoryId: string): Promise<AssayResponse[]> {
  return request<AssayResponse[]>(`/repositories/${repositoryId}/assays`);
}

export function getOrRunAssay(
  repositoryId: string,
  lookup: AssayLookupRequest,
): Promise<AssayResponse> {
  const params = new URLSearchParams({
    ecosystem: lookup.ecosystem,
    name: lookup.name,
    version: lookup.version,
  });
  return request<AssayResponse>(`/repositories/${repositoryId}/assay?${params.toString()}`);
}

export function runAssay(
  repositoryId: string,
  lookup: AssayLookupRequest,
): Promise<AssayResponse> {
  return request<AssayResponse>(`/repositories/${repositoryId}/assays`, {
    method: "POST",
    body: JSON.stringify(lookup),
  });
}

export function rerunAllAssays(): Promise<AssayRerunResponse> {
  return request<AssayRerunResponse>("/assays/rerun", { method: "POST" });
}

export function getRetentionPolicy(repositoryId: string): Promise<RetentionPolicyResponse> {
  return request<RetentionPolicyResponse>(`/repositories/${repositoryId}/retention`);
}

export function saveRetentionPolicy(
  repositoryId: string,
  payload: RetentionPolicyRequest,
): Promise<RetentionPolicyResponse> {
  return request<RetentionPolicyResponse>(`/repositories/${repositoryId}/retention`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function dryRunRetention(
  repositoryId: string,
  payload: RetentionPolicyRequest,
): Promise<CleanupPreviewResponse> {
  return request<CleanupPreviewResponse>(
    `/repositories/${repositoryId}/retention/dry-run`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function applyRetention(
  repositoryId: string,
  payload: RetentionPolicyRequest,
): Promise<CleanupPreviewResponse> {
  return request<CleanupPreviewResponse>(`/repositories/${repositoryId}/retention/apply`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function collectGarbage(repositoryId: string): Promise<CleanupReportResponse> {
  return request<CleanupReportResponse>(`/repositories/${repositoryId}/gc`, {
    method: "POST",
  });
}

export async function downloadAssaySbom(assayId: string, filename: string): Promise<void> {
  const response = await fetch(`${API_BASE_URL}/assays/${assayId}/sbom`, {
    headers: authHeaders(),
  });
  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }
  const blob = await response.blob();
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(objectUrl);
}
