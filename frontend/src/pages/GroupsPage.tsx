import { AlertCircle, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link, Navigate, useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useDeleteGroup, useGroups } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers } from "@/auth/roles";
import { CreateGroupDialog } from "@/components/groups/CreateGroupDialog";
import { NameChips } from "@/components/groups/NameChips";
import { PageHeader } from "@/components/layout/PageHeader";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export function GroupsPage() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const navigate = useNavigate();
  const { data, isPending, isError, error, refetch, isFetching } = useGroups();
  const deleteGroup = useDeleteGroup();

  if (!canManageUsers(user?.role)) {
    return <Navigate to="/repositories" replace />;
  }

  async function onDelete(groupId: string, name: string) {
    try {
      await deleteGroup.mutateAsync(groupId);
      toast.success(t("groups.deleted", { name }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("groups.deleteFailed"));
      throw err;
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={t("groups.title")}
        description={t("groups.description")}
        actions={<CreateGroupDialog />}
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
          <AlertTitle>{t("groups.loadFailed")}</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            <Button size="sm" variant="outline" onClick={() => void refetch()}>
              <RefreshCw className={isFetching ? "animate-spin" : ""} />
              {t("common.retry")}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {data && data.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("groups.noneYet")}
        </p>
      ) : null}

      {data && data.length > 0 ? (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("groups.title")}</TableHead>
                <TableHead>{t("common.members")}</TableHead>
                <TableHead>{t("nav.repositories")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((group) => (
                <TableRow
                  key={group.id}
                  className="cursor-pointer"
                  onClick={() => navigate(`/groups/${group.id}`)}
                >
                  <TableCell className="font-medium">
                    <Link
                      to={`/groups/${group.id}`}
                      className="hover:underline"
                      onClick={(event) => event.stopPropagation()}
                    >
                      {group.name}
                    </Link>
                  </TableCell>
                  <TableCell className="whitespace-normal">
                    <NameChips names={group.member_names} empty={t("common.none")} />
                  </TableCell>
                  <TableCell className="whitespace-normal">
                    <NameChips names={group.repository_names} empty={t("common.none")} />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={(event) => {
                          event.stopPropagation();
                          navigate(`/groups/${group.id}?edit=1`);
                        }}
                      >
                        <Pencil />
                        {t("common.edit")}
                      </Button>
                      <ConfirmDeleteDialog
                        title={t("repositories.deleteTitle", { name: group.name })}
                        description={t("groups.deleteBody")}
                        pending={deleteGroup.isPending}
                        onConfirm={() => onDelete(group.id, group.name)}
                        trigger={
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={(event) => event.stopPropagation()}
                          >
                            <Trash2 />
                            {t("common.delete")}
                          </Button>
                        }
                      />
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}
