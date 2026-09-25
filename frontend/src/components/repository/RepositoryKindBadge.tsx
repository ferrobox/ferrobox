import { Layers, Radar, Warehouse } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import type { RepositoryKindDto } from "@/api/generated/RepositoryKindDto";

export type RepositoryStorageKind = RepositoryKindDto["type"];

export const KIND_META = {
  forge: {
    label: "Forge",
    hintKey: "kinds.forgeHint",
    icon: Warehouse,
  },
  mirror: {
    label: "Mirror",
    hintKey: "kinds.mirrorHint",
    icon: Radar,
  },
  alloy: {
    label: "Alloy",
    hintKey: "kinds.alloyHint",
    icon: Layers,
  },
} as const;

export const KIND_FILTERS: readonly RepositoryStorageKind[] = ["forge", "mirror", "alloy"];

export function RepositoryKindBadge({ kind }: { kind: RepositoryKindDto }) {
  const { t } = useTranslation();
  const meta = KIND_META[kind.type];
  const Icon = meta.icon;

  return (
    <Badge variant="secondary" className="gap-1.5 font-medium" title={t(meta.hintKey)}>
      <Icon className="size-3.5" />
      {meta.label}
    </Badge>
  );
}
