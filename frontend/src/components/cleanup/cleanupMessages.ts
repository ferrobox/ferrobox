import type { TFunction } from "i18next";

import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import { formatBytes } from "@/lib/format";

export function previewCatalogMessage(
  preview: CleanupPreviewResponse,
  t: TFunction,
): string {
  if (preview.items.length === 0) {
    return t("gc.previewEmpty");
  }
  return t("gc.previewDrop", { count: preview.dropped_versions });
}

export function appliedCatalogMessage(
  preview: CleanupPreviewResponse,
  t: TFunction,
): string {
  return t("gc.appliedDrop", { count: preview.dropped_versions });
}

export function previewGarbageMessage(
  preview: CleanupPreviewResponse,
  t: TFunction,
): string {
  if (preview.items.length === 0) {
    return t("gc.previewOrphansEmpty");
  }
  return t("gc.previewOrphans", {
    count: preview.deleted_artifacts,
    size: formatBytes(preview.freed_bytes),
  });
}

export function appliedGarbageMessage(
  preview: CleanupPreviewResponse,
  t: TFunction,
): string {
  return t("gc.appliedOrphans", {
    count: preview.deleted_artifacts,
    size: formatBytes(preview.freed_bytes),
  });
}
