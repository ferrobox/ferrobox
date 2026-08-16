import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import * as api from "@/api/client";
import type { AssayLookupRequest } from "@/api/generated/AssayLookupRequest";
import type { ChangePasswordRequest } from "@/api/generated/ChangePasswordRequest";
import type { CreateApiTokenRequest } from "@/api/generated/CreateApiTokenRequest";
import type { CreateRepositoryRequest } from "@/api/generated/CreateRepositoryRequest";
import type { CreateUserRequest } from "@/api/generated/CreateUserRequest";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import type { RoleDto } from "@/api/generated/RoleDto";
import type { UpdateAlloyMembersRequest } from "@/api/generated/UpdateAlloyMembersRequest";
import type { QuotaRequest } from "@/api/generated/QuotaRequest";
import type { RetentionPolicyRequest } from "@/api/generated/RetentionPolicyRequest";

export const queryKeys = {
  repositories: ["repositories"] as const,
  repository: (id: string) => ["repositories", id] as const,
  repositoryArtifacts: (id: string) => ["repositories", id, "artifacts"] as const,
  apiTokens: ["auth", "tokens"] as const,
  users: ["users"] as const,
  settings: ["settings"] as const,
  assays: ["assays"] as const,
  repositoryAssays: (id: string) => ["repositories", id, "assays"] as const,
  repositoryRetention: (id: string) => ["repositories", id, "retention"] as const,
  repositoryQuota: (id: string) => ["repositories", id, "quota"] as const,
};

export function useRepositories() {
  return useQuery({
    queryKey: queryKeys.repositories,
    queryFn: api.listRepositories,
  });
}

export function useRepository(repositoryId: string) {
  return useQuery({
    queryKey: queryKeys.repository(repositoryId),
    queryFn: () => api.getRepository(repositoryId),
  });
}

export function useRepositoryArtifacts(repositoryId: string) {
  return useQuery({
    queryKey: queryKeys.repositoryArtifacts(repositoryId),
    queryFn: () => api.listRepositoryArtifacts(repositoryId),
  });
}

export function useCargoRegistryConfig(repositoryId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["repositories", repositoryId, "cargo-config"],
    queryFn: () => api.getCargoRegistryConfig(repositoryId),
    enabled,
    retry: false,
  });
}

export function useApiTokens() {
  return useQuery({
    queryKey: queryKeys.apiTokens,
    queryFn: api.listApiTokens,
  });
}

export function useCreateRepository() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: CreateRepositoryRequest) => api.createRepository(payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.repositories });
    },
  });
}

export function useDeleteRepository() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (repositoryId: string) => api.deleteRepository(repositoryId),
    onSuccess: (_data, repositoryId) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.repositories });
      void queryClient.removeQueries({ queryKey: queryKeys.repository(repositoryId) });
      void queryClient.removeQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
    },
  });
}

export function useUpdateAlloyMembers(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: UpdateAlloyMembersRequest) =>
      api.updateAlloyMembers(repositoryId, payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.repository(repositoryId) });
      void queryClient.invalidateQueries({ queryKey: queryKeys.repositories });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
    },
  });
}

export function usePublishArtifact(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (file: File) => api.publishArtifact(repositoryId, file),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
    },
  });
}

export function useDeleteArtifact(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (artifactId: string) => api.deleteArtifact(repositoryId, artifactId),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
    },
  });
}

export function useSetYanked(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      name,
      version,
      yanked,
      ecosystem,
    }: {
      name: string;
      version: string;
      yanked: boolean;
      ecosystem: PackageEcosystemDto;
    }) => {
      if (ecosystem === "npm") {
        return yanked
          ? api.yankNpm(repositoryId, name, version)
          : api.unyankNpm(repositoryId, name, version);
      }
      if (ecosystem === "pypi") {
        return yanked
          ? api.yankPypi(repositoryId, name, version)
          : api.unyankPypi(repositoryId, name, version);
      }
      if (ecosystem === "oci" || ecosystem === "helm") {
        return yanked
          ? api.yankOci(repositoryId, name, version)
          : api.unyankOci(repositoryId, name, version);
      }
      if (ecosystem === "conan") {
        return yanked
          ? api.yankConan(repositoryId, name, version)
          : api.unyankConan(repositoryId, name, version);
      }
      return yanked
        ? api.yankCrate(repositoryId, name, version)
        : api.unyankCrate(repositoryId, name, version);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
    },
  });
}

export function useCreateApiToken() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: CreateApiTokenRequest) => api.createApiToken(payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.apiTokens });
    },
  });
}

export function useRevokeApiToken() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (tokenId: string) => api.revokeApiToken(tokenId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.apiTokens });
    },
  });
}

export function useUsers() {
  return useQuery({
    queryKey: queryKeys.users,
    queryFn: api.listUsers,
  });
}

export function useCreateUser() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: CreateUserRequest) => api.createUser(payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.users });
    },
  });
}

export function useDeleteUser() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (userId: string) => api.deleteUser(userId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.users });
    },
  });
}

export function useUpdateUserRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: RoleDto }) =>
      api.updateUserRole(userId, { role }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.users });
    },
  });
}

export function useSettings() {
  return useQuery({
    queryKey: queryKeys.settings,
    queryFn: api.getSettings,
  });
}

export function useChangePassword() {
  return useMutation({
    mutationFn: (payload: ChangePasswordRequest) => api.changePassword(payload),
  });
}

export function useAssays() {
  return useQuery({
    queryKey: queryKeys.assays,
    queryFn: api.listAssays,
  });
}

export function useRepositoryAssays(repositoryId: string) {
  return useQuery({
    queryKey: queryKeys.repositoryAssays(repositoryId),
    queryFn: () => api.listRepositoryAssays(repositoryId),
  });
}

export function useAssay(
  repositoryId: string,
  lookup: AssayLookupRequest | null,
  enabled: boolean,
) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: [
      "repositories",
      repositoryId,
      "assay",
      lookup?.ecosystem,
      lookup?.name,
      lookup?.version,
    ],
    queryFn: async () => {
      const assay = await api.getOrRunAssay(repositoryId, lookup as AssayLookupRequest);
      void queryClient.invalidateQueries({ queryKey: queryKeys.assays });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryAssays(repositoryId),
      });
      return assay;
    },
    enabled: enabled && lookup !== null,
  });
}

export function useRunAssay(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (lookup: AssayLookupRequest) => api.runAssay(repositoryId, lookup),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.assays });
      void queryClient.invalidateQueries({ queryKey: queryKeys.repositoryAssays(repositoryId) });
      void queryClient.invalidateQueries({ queryKey: ["repositories", repositoryId, "assay"] });
    },
  });
}

export function useRerunAllAssays() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: api.rerunAllAssays,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.assays });
      void queryClient.invalidateQueries({ queryKey: ["repositories"] });
    },
  });
}

export function useRetentionPolicy(repositoryId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.repositoryRetention(repositoryId),
    queryFn: () => api.getRetentionPolicy(repositoryId),
    enabled,
  });
}

export function useSaveRetentionPolicy(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: RetentionPolicyRequest) =>
      api.saveRetentionPolicy(repositoryId, payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryRetention(repositoryId),
      });
    },
  });
}

export function useDryRunRetention(repositoryId: string) {
  return useMutation({
    mutationFn: (payload: RetentionPolicyRequest) =>
      api.dryRunRetention(repositoryId, payload),
  });
}

export function useApplyRetention(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: RetentionPolicyRequest) =>
      api.applyRetention(repositoryId, payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryRetention(repositoryId),
      });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.assays });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryAssays(repositoryId),
      });
    },
  });
}

export function useCollectGarbage(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => api.collectGarbage(repositoryId),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryArtifacts(repositoryId),
      });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryQuota(repositoryId),
      });
    },
  });
}

export function useDryRunGarbageCollection() {
  return useMutation({
    mutationFn: api.dryRunGarbageCollection,
  });
}

export function useCollectGarbageAll() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: api.collectGarbageAll,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.repositories });
      void queryClient.invalidateQueries({ queryKey: queryKeys.assays });
    },
  });
}

export function useQuota(repositoryId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.repositoryQuota(repositoryId),
    queryFn: () => api.getQuota(repositoryId),
    enabled,
  });
}

export function useSaveQuota(repositoryId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: QuotaRequest) => api.saveQuota(repositoryId, payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryQuota(repositoryId),
      });
    },
  });
}
