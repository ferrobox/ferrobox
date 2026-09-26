import { type ChangeEvent, useRef } from "react";
import { Download, Loader2, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useExportRepository, useImportRepository } from "@/api/queries";
import { Button } from "@/components/ui/button";

export function RepositoryBundleButtons({
  repositoryId,
  kind,
  canWrite,
}: {
  repositoryId: string;
  kind: "forge" | "mirror" | "alloy";
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLInputElement>(null);
  const exportBundle = useExportRepository(repositoryId);
  const importBundle = useImportRepository(repositoryId);

  async function onExport() {
    try {
      const { blob, filename } = await exportBundle.mutateAsync();
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = filename;
      link.click();
      URL.revokeObjectURL(url);
      toast.success(t("bundle.exported", { name: filename }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("bundle.exportFailed"));
    }
  }

  function onImport(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) {
      return;
    }
    importBundle.mutate(file, {
      onSuccess: (result) => {
        toast.success(
          t("bundle.imported", {
            packages: result.packages_imported,
            artifacts: result.artifacts_imported,
          }),
        );
      },
      onError: (error) => {
        toast.error(error instanceof ApiError ? error.message : t("bundle.importFailed"));
      },
    });
  }

  if (kind === "alloy") {
    return null;
  }

  return (
    <>
      <input
        ref={inputRef}
        type="file"
        accept=".tar.gz,.tgz,application/gzip"
        className="hidden"
        onChange={onImport}
      />
      <Button
        type="button"
        variant="outline"
        onClick={() => void onExport()}
        disabled={exportBundle.isPending}
      >
        {exportBundle.isPending ? <Loader2 className="animate-spin" /> : <Download />}
        {t("bundle.export")}
      </Button>
      {canWrite && kind === "forge" ? (
        <Button
          type="button"
          variant="outline"
          onClick={() => inputRef.current?.click()}
          disabled={importBundle.isPending}
        >
          {importBundle.isPending ? <Loader2 className="animate-spin" /> : <Upload />}
          {t("bundle.import")}
        </Button>
      ) : null}
    </>
  );
}
