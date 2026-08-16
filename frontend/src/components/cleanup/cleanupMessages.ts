import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import { formatBytes } from "@/lib/format";

export function previewCatalogMessage(preview: CleanupPreviewResponse): string {
  if (preview.items.length === 0) {
    return "Nada que sacar del catálogo con esta política.";
  }
  return `Se sacarían ${String(preview.dropped_versions)} versiones del catálogo. El disco no se libera hasta la recolección de basura.`;
}

export function appliedCatalogMessage(preview: CleanupPreviewResponse): string {
  return `Sacadas ${String(preview.dropped_versions)} versiones del catálogo. El espacio en disco se libera en Configuración → Recolección de basura.`;
}

export function previewGarbageMessage(preview: CleanupPreviewResponse): string {
  if (preview.items.length === 0) {
    return "No hay binarios huérfanos que borrar.";
  }
  return `Se borrarían ${String(preview.deleted_artifacts)} binarios (${formatBytes(preview.freed_bytes)}).`;
}

export function appliedGarbageMessage(preview: CleanupPreviewResponse): string {
  return `Borrados ${String(preview.deleted_artifacts)} binarios (${formatBytes(preview.freed_bytes)}).`;
}
