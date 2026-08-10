import type { ArtifactResponse } from "@/api/generated/ArtifactResponse";
import type { CreateRepositoryRequest } from "@/api/generated/CreateRepositoryRequest";
import type { CreateRepositoryResponse } from "@/api/generated/CreateRepositoryResponse";
import type { ErrorResponse } from "@/api/generated/ErrorResponse";
import type { PublishResponse } from "@/api/generated/PublishResponse";
import type { RepositoryResponse } from "@/api/generated/RepositoryResponse";

/**
 * Todas las peticiones se dirigen a `/api`, que el servidor de
 * desarrollo de Vite reenvía a `http://127.0.0.1:3000` (ver
 * `vite.config.ts`), evitando problemas de CORS sin necesidad de
 * codificar ninguna URL absoluta en el cliente.
 */
const API_BASE_URL = "/api";

/** Error tipado lanzado por el cliente HTTP ante cualquier respuesta no exitosa. */
export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    headers: { "content-type": "application/json", ...init?.headers },
    ...init,
  });

  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
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
    headers: { "content-type": "application/octet-stream" },
    body: file,
  });

  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }

  return (await response.json()) as PublishResponse;
}

/** URL de descarga directa de un artefacto -- se usa como `href`, no vía `fetch`. */
export function artifactDownloadUrl(artifactId: string): string {
  return `${API_BASE_URL}/artifacts/${artifactId}`;
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
