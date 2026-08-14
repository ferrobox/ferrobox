import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import * as api from "@/api/client";
import type { ChangePasswordRequest } from "@/api/generated/ChangePasswordRequest";
import type { CreateApiTokenRequest } from "@/api/generated/CreateApiTokenRequest";
import type { CreateRepositoryRequest } from "@/api/generated/CreateRepositoryRequest";
import type { CreateUserRequest } from "@/api/generated/CreateUserRequest";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import type { RoleDto } from "@/api/generated/RoleDto";
import type { UpdateAlloyMembersRequest } from "@/api/generated/UpdateAlloyMembersRequest";

export const queryKeys = {
  repositories: ["repositories"] as const,
  repository: (id: string) => ["repositories", id] as const,
  repositoryArtifacts: (id: string) => ["repositories", id, "artifacts"] as const,
  apiTokens: ["auth", "tokens"] as const,
  users: ["users"] as const,
  settings: ["settings"] as const,
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
