import { useMemo, useState } from "react";
import { AlertCircle, ArrowLeft, Loader2, RefreshCw } from "lucide-react";
import { Link, Navigate, useParams } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useGroup, useRepositories, useSaveGroup, useUsers } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
import { KIND_META } from "@/components/repository/RepositoryKindBadge";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import NotFoundPage from "@/pages/NotFoundPage";

const GROUP_ROLES: readonly Exclude<RoleDto, "admin">[] = ["reader", "developer"];

export function GroupDetailPage() {
  const { groupId } = useParams<{ groupId: string }>();
  const { user } = useAuth();

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }
  if (!groupId) {
    return <NotFoundPage />;
  }

  return <GroupDetailContent groupId={groupId} />;
}

function GroupDetailContent({ groupId }: { groupId: string }) {
  const groupQuery = useGroup(groupId);
  const usersQuery = useUsers();
  const repositoriesQuery = useRepositories();
  const saveGroup = useSaveGroup(groupId);

  const [draftUserIds, setDraftUserIds] = useState<string[] | null>(null);
  const [draftRepoRoles, setDraftRepoRoles] = useState<Record<string, RoleDto | "none"> | null>(
    null,
  );

  const selectedUserIds =
    draftUserIds ?? groupQuery.data?.members.map((member) => member.id) ?? [];
  const repoRoles =
    draftRepoRoles ??
    Object.fromEntries(
      (groupQuery.data?.repositories ?? []).map((grant) => [grant.repository_id, grant.role]),
    );

  const users = useMemo(
    () =>
      (usersQuery.data ?? [])
        .slice()
        .sort((left, right) => left.username.localeCompare(right.username)),
    [usersQuery.data],
  );
  const repositories = useMemo(
    () =>
      (repositoriesQuery.data ?? [])
        .slice()
        .sort((left, right) => left.name.localeCompare(right.name)),
    [repositoriesQuery.data],
  );

  if (groupQuery.isPending) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-16 w-full rounded-xl" />
        <Skeleton className="h-64 w-full rounded-xl" />
      </div>
    );
  }

  if (groupQuery.isError) {
    if (groupQuery.error instanceof ApiError && groupQuery.error.status === 404) {
      return <NotFoundPage message="Ese grupo no existe." />;
    }
    return (
      <Alert variant="destructive">
        <AlertCircle />
        <AlertTitle>No se pudo cargar el grupo</AlertTitle>
        <AlertDescription className="flex items-center justify-between gap-4">
          <span>{groupQuery.error.message}</span>
          <Button size="sm" variant="outline" onClick={() => void groupQuery.refetch()}>
            <RefreshCw className={groupQuery.isFetching ? "animate-spin" : ""} />
            Reintentar
          </Button>
        </AlertDescription>
      </Alert>
    );
  }

  const group = groupQuery.data;

  function toggleUser(userId: string) {
    setDraftUserIds((current) => {
      const selected = current ?? selectedUserIds;
      return selected.includes(userId)
        ? selected.filter((id) => id !== userId)
        : [...selected, userId];
    });
  }

  async function onSave() {
    const grants = Object.entries(repoRoles)
      .filter((entry): entry is [string, Exclude<RoleDto, "admin">] => {
        const role = entry[1];
        return role === "reader" || role === "developer";
      })
      .map(([repository_id, role]) => ({ repository_id, role }));
    try {
      await saveGroup.mutateAsync({
        members: { user_ids: selectedUserIds },
        repositories: { grants },
      });
      setDraftUserIds(null);
      setDraftRepoRoles(null);
      toast.success("Grupo actualizado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo guardar el grupo");
    }
  }

  return (
    <div className="space-y-8">
      <div>
        <Button variant="ghost" size="sm" asChild className="mb-3 -ml-2">
          <Link to="/groups">
            <ArrowLeft />
            Grupos
          </Link>
        </Button>
        <PageHeader
          title={group.name}
          description="Elige los miembros y los repositorios. Quien esté en este grupo solo verá esos repositorios, con rol Lector o Desarrollador."
          actions={
            <Button onClick={() => void onSave()} disabled={saveGroup.isPending}>
              {saveGroup.isPending ? <Loader2 className="animate-spin" /> : null}
              Guardar
            </Button>
          }
        />
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Miembros</CardTitle>
            <CardDescription>Los usuarios marcados pertenecen a este grupo.</CardDescription>
          </CardHeader>
          <CardContent>
            {users.length === 0 ? (
              <p className="text-sm text-muted-foreground">No hay usuarios en la instancia.</p>
            ) : (
              <ul className="max-h-80 space-y-1 overflow-y-auto rounded-md border border-border p-2">
                {users.map((entry) => (
                  <li key={entry.id}>
                    <label className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-muted">
                      <input
                        type="checkbox"
                        className="size-4 accent-primary"
                        checked={selectedUserIds.includes(entry.id)}
                        onChange={() => toggleUser(entry.id)}
                      />
                      <span className="min-w-0 flex-1 truncate font-medium">{entry.username}</span>
                      <span className="text-xs text-muted-foreground">{roleLabel(entry.role)}</span>
                    </label>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Repositorios</CardTitle>
            <CardDescription>
              Asigna un rol por repositorio. «Sin acceso» deja ese repositorio fuera del grupo.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {repositories.length === 0 ? (
              <p className="text-sm text-muted-foreground">No hay repositorios.</p>
            ) : (
              <ul className="max-h-80 space-y-2 overflow-y-auto rounded-md border border-border p-2">
                {repositories.map((repository) => {
                  const role = repoRoles[repository.id] ?? "none";
                  const kindLabel = KIND_META[repository.kind.type].label;
                  return (
                    <li
                      key={repository.id}
                      className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm"
                    >
                      <span className="min-w-0 flex-1 truncate font-medium">{repository.name}</span>
                      <span className="text-xs text-muted-foreground">{kindLabel}</span>
                      <Select
                        value={role}
                        onValueChange={(value) => {
                          setDraftRepoRoles((current) => ({
                            ...(current ?? repoRoles),
                            [repository.id]: value as RoleDto | "none",
                          }));
                        }}
                      >
                        <SelectTrigger
                          aria-label={`Rol de ${group.name} en ${repository.name}`}
                          className="h-8 w-[150px]"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="none">Sin acceso</SelectItem>
                          {GROUP_ROLES.map((option) => (
                            <SelectItem key={option} value={option}>
                              {roleLabel(option)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </li>
                  );
                })}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
