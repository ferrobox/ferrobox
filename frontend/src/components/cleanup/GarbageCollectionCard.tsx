import { useState } from "react";
import { Eraser, FlaskConical } from "lucide-react";
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
  const { user } = useAuth();
  const canWrite = canWriteArtifacts(user?.role);
  const dryRun = useDryRunGarbageCollection();
  const collect = useCollectGarbageAll();
  const [preview, setPreview] = useState<CleanupPreviewResponse | null>(null);

  async function onSimulate() {
    try {
      const result = await dryRun.mutateAsync();
      setPreview(result);
      toast.success(previewGarbageMessage(result));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo simular la recolección");
    }
  }

  async function onCollect() {
    try {
      const result = await collect.mutateAsync();
      setPreview(result);
      toast.success(appliedGarbageMessage(result));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo recolectar la basura");
      throw err;
    }
  }

  return (
    <Card className="lg:col-span-2">
      <CardHeader>
        <CardTitle>Recolección de basura</CardTitle>
        <CardDescription>
          Borra de disco los binarios que ya no están en ningún catálogo (versiones sacadas por
          retención, restos de un Eliminar, capas OCI sin manifiesto). Simular no toca nada.
          Install y publish siguen disponibles mientras corre.
        </CardDescription>
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
              {dryRun.isPending ? "Simulando…" : "Simular"}
            </Button>
            <ConfirmDeleteDialog
              title="Recolectar basura"
              description={
                preview === null
                  ? "Se borrarán de disco los binarios huérfanos de toda la instancia. Simula primero para ver la lista. Esta acción no se puede deshacer."
                  : `${previewGarbageMessage(preview)} Esta acción no se puede deshacer.`
              }
              confirmLabel="Recolectar"
              pending={collect.isPending}
              onConfirm={onCollect}
              trigger={
                <Button type="button" variant="destructive" disabled={preview === null}>
                  <Eraser />
                  Recolectar ahora
                </Button>
              }
            />
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            Solo un usuario Developer o Admin puede simular y recolectar.
          </p>
        )}
        {preview ? (
          <CleanupPreviewTable
            preview={preview}
            showRepository
            appliedLabel="Recolectado"
            summary={
              preview.dry_run ? previewGarbageMessage(preview) : appliedGarbageMessage(preview)
            }
          />
        ) : null}
      </CardContent>
    </Card>
  );
}
