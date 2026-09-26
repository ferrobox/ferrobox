import { AlertCircle, KeyRound, RefreshCw, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Navigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteUser, useUpdateUserRole, useUsers } from "@/api/queries";
import type { RoleDto } from "@/api/generated/RoleDto";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers, roleLabel } from "@/auth/roles";
import { PageHeader } from "@/components/layout/PageHeader";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { CreateRobotDialog } from "@/components/users/CreateRobotDialog";
import { CreateUserDialog } from "@/components/users/CreateUserDialog";
import { ResetPasswordDialog } from "@/components/users/ResetPasswordDialog";
import { RolePermissionsCard } from "@/components/users/RolePermissionsCard";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const ROLE_OPTIONS: readonly RoleDto[] = ["admin", "developer", "reader"];

export function UsersPage() {
  const { t } = useTranslation();
  const { user, updateCurrentUser } = useAuth();
  const { data, isPending, isError, error, refetch, isFetching } = useUsers();
  const deleteUser = useDeleteUser();
  const updateUserRole = useUpdateUserRole();

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  async function onChangeRole(userId: string, name: string, nextRole: RoleDto) {
    try {
      const updated = await updateUserRole.mutateAsync({ userId, role: nextRole });
      if (updated.id === user?.id) {
        updateCurrentUser(updated);
      }
      toast.success(t("users.roleUpdated", { name }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("users.roleFailed"));
    }
  }

  async function onDelete(userId: string, name: string) {
    try {
      await deleteUser.mutateAsync(userId);
      toast.success(t("users.deleted", { name }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("users.deleteFailed"));
      throw err;
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={t("users.title")}
        description={t("users.description")}
        actions={
          <div className="flex gap-2">
            <CreateRobotDialog />
            <CreateUserDialog />
          </div>
        }
      />

      {isPending ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full rounded-lg" />
          ))}
        </div>
      ) : null}

      {isError ? (
        <Alert variant="destructive">
          <AlertCircle />
          <AlertTitle>{t("users.loadFailed")}</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              {t("common.retry")}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {data ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("users.username")}</TableHead>
                <TableHead>{t("users.email")}</TableHead>
                <TableHead>{t("users.role")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((entry) => {
                const isSelf = entry.id === user?.id;
                return (
                  <TableRow key={entry.id}>
                    <TableCell className="font-medium">
                      <span className="flex items-center gap-2">
                        {entry.username}
                        {entry.sso ? (
                          <Badge variant="secondary">{t("users.sso")}</Badge>
                        ) : null}
                        {entry.robot ? (
                          <Badge variant="secondary">{t("users.robot")}</Badge>
                        ) : null}
                      </span>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {entry.email ?? "—"}
                    </TableCell>
                    <TableCell>
                      <Select
                        value={entry.role}
                        disabled={updateUserRole.isPending}
                        onValueChange={(value) => {
                          const nextRole = value as RoleDto;
                          if (nextRole === entry.role) {
                            return;
                          }
                          void onChangeRole(entry.id, entry.username, nextRole);
                        }}
                      >
                        <SelectTrigger
                          aria-label={t("users.roleOf", { name: entry.username })}
                          className="h-8 w-[140px]"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {(entry.robot
                            ? ROLE_OPTIONS.filter((option) => option !== "admin")
                            : ROLE_OPTIONS
                          ).map((option) => (
                            <SelectItem key={option} value={option}>
                              {roleLabel(option)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-1">
                        {entry.robot ? null : (
                          <ResetPasswordDialog
                            userId={entry.id}
                            username={entry.username}
                            trigger={
                              <Button variant="ghost" size="sm" disabled={isSelf}>
                                <KeyRound />
                                {t("users.reset")}
                              </Button>
                            }
                          />
                        )}
                        <ConfirmDeleteDialog
                          title={t("repositories.deleteTitle", { name: entry.username })}
                          description={t("users.deleteTokens")}
                          pending={deleteUser.isPending}
                          onConfirm={() => onDelete(entry.id, entry.username)}
                          trigger={
                            <Button variant="ghost" size="sm" disabled={isSelf}>
                              <Trash2 />
                              {t("common.delete")}
                            </Button>
                          }
                        />
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      ) : null}

      <RolePermissionsCard />
    </div>
  );
}
