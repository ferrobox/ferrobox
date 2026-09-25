import { Boxes } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { CreateRepositoryDialog } from "@/components/repository/CreateRepositoryDialog";

export function RepositoriesPage() {
  const { user } = useAuth();
  const { t } = useTranslation();
  const canWrite = canWriteArtifacts(user?.role);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
      <div className="flex size-12 items-center justify-center rounded-full bg-muted">
        <Boxes className="size-6 text-muted-foreground" />
      </div>
      <div className="max-w-md space-y-1">
        <h1 className="text-xl font-semibold text-foreground">{t("repositories.emptyTitle")}</h1>
        <p className="text-sm text-muted-foreground">{t("repositories.emptyBody")}</p>
      </div>
      {canWrite ? <CreateRepositoryDialog /> : null}
    </div>
  );
}
