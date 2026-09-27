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
import type { CreateRobotRequest } from "@/api/generated/CreateRobotRequest";
import type { CreateRobotResponse } from "@/api/generated/CreateRobotResponse";
import type { CreateUserRequest } from "@/api/generated/CreateUserRequest";
import type { ResetUserPasswordRequest } from "@/api/generated/ResetUserPasswordRequest";
import type { ErrorResponse } from "@/api/generated/ErrorResponse";
import type { LoginRequest } from "@/api/generated/LoginRequest";
import type { LoginResponse } from "@/api/generated/LoginResponse";
import type { PublishResponse } from "@/api/generated/PublishResponse";
import type { ImportRepositoryResponse } from "@/api/generated/ImportRepositoryResponse";
import type { PrefetchPackageRequest } from "@/api/generated/PrefetchPackageRequest";
import type { PrefetchPackageResponse } from "@/api/generated/PrefetchPackageResponse";
import type { PromotePackageRequest } from "@/api/generated/PromotePackageRequest";
import type { PromotePackageResponse } from "@/api/generated/PromotePackageResponse";
import type { RepositoryResponse } from "@/api/generated/RepositoryResponse";
import type { SetMirrorScheduleRequest } from "@/api/generated/SetMirrorScheduleRequest";
import type { QuotaRequest } from "@/api/generated/QuotaRequest";
import type { QuotaResponse } from "@/api/generated/QuotaResponse";
import type { WormRequest } from "@/api/generated/WormRequest";
import type { WormResponse } from "@/api/generated/WormResponse";
import type { SearchResponse } from "@/api/generated/SearchResponse";
import type { AdmissionEventResponse } from "@/api/generated/AdmissionEventResponse";
import type { AuditEventResponse } from "@/api/generated/AuditEventResponse";
import type { AdmissionPolicyRequest } from "@/api/generated/AdmissionPolicyRequest";
import type { AdmissionPolicyResponse } from "@/api/generated/AdmissionPolicyResponse";
import type { AdmissionPreviewResponse } from "@/api/generated/AdmissionPreviewResponse";
import type { RetentionPolicyRequest } from "@/api/generated/RetentionPolicyRequest";
import type { RetentionPolicyResponse } from "@/api/generated/RetentionPolicyResponse";
import type { ReplicaPolicyRequest } from "@/api/generated/ReplicaPolicyRequest";
import type { ReplicaPolicyResponse } from "@/api/generated/ReplicaPolicyResponse";
import type { ReplicaPushResponse } from "@/api/generated/ReplicaPushResponse";
import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import type { CleanupReportResponse } from "@/api/generated/CleanupReportResponse";
import type { SettingsResponse } from "@/api/generated/SettingsResponse";
import type { StorageResponse } from "@/api/generated/StorageResponse";
import type { OidcStatusResponse } from "@/api/generated/OidcStatusResponse";
import type { UpdateAlloyMembersRequest } from "@/api/generated/UpdateAlloyMembersRequest";
import type { UpdateUserRoleRequest } from "@/api/generated/UpdateUserRoleRequest";
import type { UserResponse } from "@/api/generated/UserResponse";
import type { GroupSummaryResponse } from "@/api/generated/GroupSummaryResponse";
import type { GroupDetailResponse } from "@/api/generated/GroupDetailResponse";
import type { MyGroupMembershipResponse } from "@/api/generated/MyGroupMembershipResponse";
import type { CreateGroupRequest } from "@/api/generated/CreateGroupRequest";
import type { SetGroupMembersRequest } from "@/api/generated/SetGroupMembersRequest";
import type { SetGroupRepositoriesRequest } from "@/api/generated/SetGroupRepositoriesRequest";
import type { RepositoryAccessGrantResponse } from "@/api/generated/RepositoryAccessGrantResponse";
import type { SetRepositoryAccessRequest } from "@/api/generated/SetRepositoryAccessRequest";
import type { CreateWebhookRequest } from "@/api/generated/CreateWebhookRequest";
import type { UpdateWebhookRequest } from "@/api/generated/UpdateWebhookRequest";
import type { WebhookDeliveryResponse } from "@/api/generated/WebhookDeliveryResponse";
import type { WebhookResponse } from "@/api/generated/WebhookResponse";

/**
 * All requests go to `/api`, which the Vite development server
 * proxies to `http://127.0.0.1:3000` (see `vite.config.ts`),
 * avoiding CORS issues without encoding any absolute URL in the
 * client.
 */
const API_BASE_URL = "/api";

const TOKEN_STORAGE_KEY = "ferrobox.auth.token";

/** Typed error thrown by the HTTP client on any unsuccessful response. */
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
    // The body was not JSON (or was empty) -- the generic message is used.
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

export function getOidcStatus(): Promise<OidcStatusResponse> {
  return request<OidcStatusResponse>("/auth/oidc");
}

export async function completeSsoSession(token: string): Promise<UserResponse> {
  setStoredToken(token);
  try {
    return await getMe();
  } catch (error) {
    setStoredToken(null);
    throw error;
  }
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

export function getStorage(): Promise<StorageResponse> {
  return request<StorageResponse>("/storage");
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

export function createRobot(payload: CreateRobotRequest): Promise<CreateRobotResponse> {
  return request<CreateRobotResponse>("/users/robots", {
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

export function resetUserPassword(
  userId: string,
  payload: ResetUserPasswordRequest,
): Promise<void> {
  return request<void>(`/users/${userId}/password`, {
    method: "POST",
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

export function promotePackage(
  repositoryId: string,
  payload: PromotePackageRequest,
): Promise<PromotePackageResponse> {
  return request<PromotePackageResponse>(`/repositories/${repositoryId}/promote`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function setMirrorSchedule(
  repositoryId: string,
  payload: SetMirrorScheduleRequest,
): Promise<RepositoryResponse> {
  return request<RepositoryResponse>(`/repositories/${repositoryId}/schedule`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export async function exportRepository(
  repositoryId: string,
): Promise<{ blob: Blob; filename: string }> {
  const response = await fetch(`${API_BASE_URL}/repositories/${repositoryId}/export`, {
    headers: authHeaders(),
  });
  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }
  const disposition = response.headers.get("content-disposition") ?? "";
  const match = /filename="([^"]+)"/.exec(disposition);
  return {
    blob: await response.blob(),
    filename: match?.[1] ?? "repository.ferrobox.tar.gz",
  };
}

export async function importRepository(
  repositoryId: string,
  file: File,
): Promise<ImportRepositoryResponse> {
  const body = new FormData();
  body.append("bundle", file);
  const response = await fetch(`${API_BASE_URL}/repositories/${repositoryId}/import`, {
    method: "POST",
    headers: authHeaders(),
    body,
  });
  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }
  return (await response.json()) as ImportRepositoryResponse;
}

export function prefetchPackage(
  repositoryId: string,
  payload: PrefetchPackageRequest,
): Promise<PrefetchPackageResponse> {
  return request<PrefetchPackageResponse>(`/repositories/${repositoryId}/prefetch`, {
    method: "POST",
    body: JSON.stringify(payload),
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

export function yankMaven(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/index/maven/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankMaven(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/index/maven/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

export function yankNuget(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/index/nuget/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankNuget(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/index/nuget/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

export function yankGo(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/index/go/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/yank`,
    { method: "DELETE" },
  );
}

export function unyankGo(
  repositoryId: string,
  name: string,
  version: string,
): Promise<void> {
  return request<void>(
    `/index/go/${repositoryId}/${encodeURIComponent(name)}/${encodeURIComponent(version)}/unyank`,
    { method: "PUT" },
  );
}

function contentDisposition(filename: string): string {
  const escaped = filename.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return `attachment; filename="${escaped}"; filename*=UTF-8''${encodeURIComponent(filename)}`;
}

export async function publishArtifact(
  repositoryId: string,
  file: File,
): Promise<PublishResponse> {
  const response = await fetch(`${API_BASE_URL}/repositories/${repositoryId}/artifacts`, {
    method: "POST",
    headers: authHeaders({
      "content-type": "application/octet-stream",
      "content-disposition": contentDisposition(file.name),
    }),
    body: file,
  });

  if (!response.ok) {
    throw new ApiError(response.status, await extractErrorMessage(response));
  }

  return (await response.json()) as PublishResponse;
}

/** Downloads an artifact with authentication and triggers a local save. */
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
 * Response of `GET /cargo/{repository_id}/config.json`. It is not
 * generated with `ts-rs` because it is a Cargo protocol detail, not a
 * management-API DTO consumed by this frontend -- but its shape is
 * stable and public (defined by the Cargo registry protocol itself),
 * so typing the response by hand is safe.
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

export function searchPackages(query: string, limit?: number): Promise<SearchResponse> {
  const params = new URLSearchParams();
  if (query.trim().length > 0) {
    params.set("q", query.trim());
  }
  if (limit != null) {
    params.set("limit", String(limit));
  }
  const suffix = params.toString();
  return request<SearchResponse>(suffix.length > 0 ? `/search?${suffix}` : "/search");
}

export function getQuota(repositoryId: string): Promise<QuotaResponse> {
  return request<QuotaResponse>(`/repositories/${repositoryId}/quota`);
}

export function saveQuota(repositoryId: string, payload: QuotaRequest): Promise<QuotaResponse> {
  return request<QuotaResponse>(`/repositories/${repositoryId}/quota`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function getWorm(repositoryId: string): Promise<WormResponse> {
  return request<WormResponse>(`/repositories/${repositoryId}/worm`);
}

export function saveWorm(repositoryId: string, payload: WormRequest): Promise<WormResponse> {
  return request<WormResponse>(`/repositories/${repositoryId}/worm`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function getAdmissionPolicy(repositoryId: string): Promise<AdmissionPolicyResponse> {
  return request<AdmissionPolicyResponse>(`/repositories/${repositoryId}/admission`);
}

export function saveAdmissionPolicy(
  repositoryId: string,
  payload: AdmissionPolicyRequest,
): Promise<AdmissionPolicyResponse> {
  return request<AdmissionPolicyResponse>(`/repositories/${repositoryId}/admission`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function dryRunAdmission(
  repositoryId: string,
  payload: AdmissionPolicyRequest,
): Promise<AdmissionPreviewResponse> {
  return request<AdmissionPreviewResponse>(`/repositories/${repositoryId}/admission/dry-run`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listAdmissionEvents(
  repositoryId: string,
): Promise<AdmissionEventResponse[]> {
  return request<AdmissionEventResponse[]>(`/repositories/${repositoryId}/admission/events`);
}

export function listAuditEvents(): Promise<AuditEventResponse[]> {
  return request<AuditEventResponse[]>("/audit");
}

export function getReplicaPolicy(repositoryId: string): Promise<ReplicaPolicyResponse> {
  return request<ReplicaPolicyResponse>(`/repositories/${repositoryId}/replica`);
}

export function saveReplicaPolicy(
  repositoryId: string,
  payload: ReplicaPolicyRequest,
): Promise<ReplicaPolicyResponse> {
  return request<ReplicaPolicyResponse>(`/repositories/${repositoryId}/replica`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function pushReplica(repositoryId: string): Promise<ReplicaPushResponse> {
  return request<ReplicaPushResponse>(`/repositories/${repositoryId}/replica/push`, {
    method: "POST",
  });
}

export function pullReplica(repositoryId: string): Promise<ReplicaPushResponse> {
  return request<ReplicaPushResponse>(`/repositories/${repositoryId}/replica/pull`, {
    method: "POST",
  });
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

export function dryRunGarbageCollection(): Promise<CleanupPreviewResponse> {
  return request<CleanupPreviewResponse>("/gc/dry-run", { method: "POST" });
}

export function collectGarbageAll(): Promise<CleanupPreviewResponse> {
  return request<CleanupPreviewResponse>("/gc", { method: "POST" });
}

export function listMyGroups(): Promise<MyGroupMembershipResponse[]> {
  return request<MyGroupMembershipResponse[]>("/auth/me/groups");
}

export function listGroups(): Promise<GroupSummaryResponse[]> {
  return request<GroupSummaryResponse[]>("/groups");
}

export function createGroup(payload: CreateGroupRequest): Promise<GroupSummaryResponse> {
  return request<GroupSummaryResponse>("/groups", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getGroup(groupId: string): Promise<GroupDetailResponse> {
  return request<GroupDetailResponse>(`/groups/${groupId}`);
}

export function deleteGroup(groupId: string): Promise<void> {
  return request<void>(`/groups/${groupId}`, { method: "DELETE" });
}

export function setGroupMembers(
  groupId: string,
  payload: SetGroupMembersRequest,
): Promise<void> {
  return request<void>(`/groups/${groupId}/members`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function setGroupRepositories(
  groupId: string,
  payload: SetGroupRepositoriesRequest,
): Promise<void> {
  return request<void>(`/groups/${groupId}/repositories`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function getRepositoryAccess(
  repositoryId: string,
): Promise<RepositoryAccessGrantResponse[]> {
  return request<RepositoryAccessGrantResponse[]>(`/repositories/${repositoryId}/access`);
}

export function listWebhooks(repositoryId: string): Promise<WebhookResponse[]> {
  return request<WebhookResponse[]>(`/repositories/${repositoryId}/webhooks`);
}

export function createWebhook(
  repositoryId: string,
  payload: CreateWebhookRequest,
): Promise<WebhookResponse> {
  return request<WebhookResponse>(`/repositories/${repositoryId}/webhooks`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function updateWebhook(
  repositoryId: string,
  webhookId: string,
  payload: UpdateWebhookRequest,
): Promise<WebhookResponse> {
  return request<WebhookResponse>(`/repositories/${repositoryId}/webhooks/${webhookId}`, {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function deleteWebhook(repositoryId: string, webhookId: string): Promise<void> {
  return request<void>(`/repositories/${repositoryId}/webhooks/${webhookId}`, {
    method: "DELETE",
  });
}

export function listWebhookDeliveries(
  repositoryId: string,
  webhookId: string,
): Promise<WebhookDeliveryResponse[]> {
  return request<WebhookDeliveryResponse[]>(
    `/repositories/${repositoryId}/webhooks/${webhookId}/deliveries`,
  );
}

export function pingWebhook(
  repositoryId: string,
  webhookId: string,
): Promise<WebhookDeliveryResponse> {
  return request<WebhookDeliveryResponse>(
    `/repositories/${repositoryId}/webhooks/${webhookId}/ping`,
    { method: "POST" },
  );
}

export function setRepositoryAccess(
  repositoryId: string,
  payload: SetRepositoryAccessRequest,
): Promise<void> {
  return request<void>(`/repositories/${repositoryId}/access`, {
    method: "PUT",
    body: JSON.stringify(payload),
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
