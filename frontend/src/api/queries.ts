import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import * as api from "@/api/client";
import type { CreateRepositoryRequest } from "@/api/generated/CreateRepositoryRequest";

export const queryKeys = {
  repositories: ["repositories"] as const,
  repository: (id: string) => ["repositories", id] as const,
  repositoryArtifacts: (id: string) => ["repositories", id, "artifacts"] as const,
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

export function useCreateRepository() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: CreateRepositoryRequest) => api.createRepository(payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.repositories });
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
