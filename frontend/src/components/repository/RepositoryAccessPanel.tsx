import { useMemo, useState } from "react";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { RoleDto } from "@/api/generated/RoleDto";
import {
  useGroups,
  useRepositoryAccess,
  useSetRepositoryAccess,
} from "@/api/queries";
import { roleLabel } from "@/auth/roles";
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

const GROUP_ROLES: readonly Exclude<RoleDto, "admin">[] = ["reader", "developer"];

export function RepositoryAccessPanel({ repositoryId }: { repositoryId: string }) {
  const { t } = useTranslation();
  const groupsQuery = useGroups();
  const accessQuery = useRepositoryAccess(repositoryId, true);
  const saveAccess = useSetRepositoryAccess(repositoryId);
  const [draft, setDraft] = useState<Record<string, RoleDto | "none"> | null>(null);
  const roles =
    draft ??
    Object.fromEntries(
      (accessQuery.data ?? []).map((grant) => [grant.group_id, grant.role]),
    );

  const groups = useMemo(
    () =>
      (groupsQuery.data ?? []).slice().sort((left, right) => left.name.localeCompare(right.name)),
    [groupsQuery.data],
  );

  async function onSave() {
    const grants = Object.entries(roles)
      .filter((entry): entry is [string, Exclude<RoleDto, "admin">] => {
        const role = entry[1];
        return role === "reader" || role === "developer";
      })
      .map(([group_id, role]) => ({ group_id, role }));
    try {
      await saveAccess.mutateAsync({ grants });
      setDraft(null);
      toast.success(
        grants.length === 0
          ? t("access.unrestricted")
          : t("access.updated"),
      );
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("access.saveFailed"));
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("access.title")}</CardTitle>
        <CardDescription>{t("access.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {groupsQuery.isPending || accessQuery.isPending ? (
          <Skeleton className="h-24 w-full rounded-md" />
        ) : null}
        {groups.length === 0 && !groupsQuery.isPending ? (
          <p className="text-sm text-muted-foreground">
            {t("access.empty")}
          </p>
        ) : (
          <ul className="max-h-64 space-y-2 overflow-y-auto rounded-md border border-border p-2">
            {groups.map((group) => {
              const role = roles[group.id] ?? "none";
              return (
                <li key={group.id} className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm">
                  <span className="min-w-0 flex-1 truncate font-medium">{group.name}</span>
                  <Select
                    value={role}
                    onValueChange={(value) => {
                      setDraft((current) => ({
                        ...(current ?? roles),
                        [group.id]: value as RoleDto | "none",
                      }));
                    }}
                  >
                    <SelectTrigger
                      aria-label={t("access.roleOf", { name: group.name })}
                      className="h-8 w-[150px]"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">{t("common.noAccess")}</SelectItem>
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
        <Button onClick={() => void onSave()} disabled={saveAccess.isPending || groups.length === 0}>
          {saveAccess.isPending ? <Loader2 className="animate-spin" /> : null}
          {t("access.save")}
        </Button>
      </CardContent>
    </Card>
  );
}
