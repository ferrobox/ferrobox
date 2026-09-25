import { useState } from "react";
import { Eraser, FlaskConical } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import { useCollectGarbageAll, useDryRunGarbageCollection } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { canWriteArtifacts } from "@/auth/roles";
import { CleanupPreviewTable } from "@/components/cleanup/CleanupPreviewTable";
import {
  appliedGarbageMessage,
  previewGarbageMessage,
} from "@/components/cleanup/cleanupMessages";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

export function GarbageCollectionCard() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const canWrite = canWriteArtifacts(user?.role);
  const dryRun = useDryRunGarbageCollection();
  const collect = useCollectGarbageAll();
  const [preview, setPreview] = useState<CleanupPreviewResponse | null>(null);

  async function onSimulate() {
    try {
      const result = await dryRun.mutateAsync();
      setPreview(result);
      toast.success(previewGarbageMessage(result, t));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("gc.simulateFailed"));
    }
  }

  async function onCollect() {
    try {
      const result = await collect.mutateAsync();
      setPreview(result);
      toast.success(appliedGarbageMessage(result, t));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("gc.applyFailed"));
      throw err;
    }
  }

  return (
    <Card className="lg:col-span-2">
      <CardHeader>
        <CardTitle>{t("gc.title")}</CardTitle>
        <CardDescription>{t("gc.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {canWrite ? (
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={dryRun.isPending}
              onClick={() => void onSimulate()}
            >
              <FlaskConical />
              {dryRun.isPending ? t("common.simulating") : t("common.simulate")}
            </Button>
            <ConfirmDeleteDialog
              title={t("gc.collectTitle")}
              description={
                preview === null
                  ? t("gc.applyHint")
                  : t("gc.applyHintPreview", { preview: previewGarbageMessage(preview, t) })
              }
              confirmLabel={t("gc.collect")}
              pending={collect.isPending}
              onConfirm={onCollect}
              trigger={
                <Button type="button" variant="destructive" disabled={preview === null}>
                  <Eraser />
                  {t("gc.collectNow")}
                </Button>
              }
            />
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t("gc.readOnly")}
          </p>
        )}
        {preview ? (
          <CleanupPreviewTable
            preview={preview}
            showRepository
            appliedLabel={t("gc.collected")}
            summary={
              preview.dry_run ? previewGarbageMessage(preview, t) : appliedGarbageMessage(preview, t)
            }
          />
        ) : null}
      </CardContent>
    </Card>
  );
}
