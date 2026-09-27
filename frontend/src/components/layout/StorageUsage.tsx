import { HardDrive } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { useStorage } from "@/api/queries";
import { formatBytes } from "@/lib/format";
import { Skeleton } from "@/components/ui/skeleton";

export function StorageUsage() {
  const { t } = useTranslation();
  const { data, isPending } = useStorage();
  const used = data == null ? "—" : formatBytes(data.used_bytes);

  return (
    <Link
      to="/settings"
      className="flex shrink-0 items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
      title={t("storage.usedHint")}
      aria-label={t("storage.usedAria", { used })}
    >
      <HardDrive className="size-4" />
      {isPending && data == null ? (
        <Skeleton className="h-4 w-16" />
      ) : (
        <span className="tabular-nums">{t("storage.used", { used })}</span>
      )}
    </Link>
  );
}
