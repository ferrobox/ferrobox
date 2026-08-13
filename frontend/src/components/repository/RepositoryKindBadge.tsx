import { Layers, Radar, Warehouse } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import type { RepositoryKindDto } from "@/api/generated/RepositoryKindDto";

export type RepositoryStorageKind = RepositoryKindDto["type"];

export const KIND_META = {
  forge: {
    label: "Forge",
    description: "Almacenamiento propio: publicas tú los artefactos",
    icon: Warehouse,
  },
  mirror: {
    label: "Mirror",
    description: "Caché pull-through de un registro externo",
    icon: Radar,
  },
  alloy: {
    label: "Alloy",
    description: "Agrega otros repositorios en una sola URL",
    icon: Layers,
  },
} as const;

export const KIND_FILTERS: readonly RepositoryStorageKind[] = ["forge", "mirror", "alloy"];

export function RepositoryKindBadge({ kind }: { kind: RepositoryKindDto }) {
  const meta = KIND_META[kind.type];
  const Icon = meta.icon;

  return (
    <Badge variant="secondary" className="gap-1.5 font-medium" title={meta.description}>
      <Icon className="size-3.5" />
      {meta.label}
    </Badge>
  );
}
