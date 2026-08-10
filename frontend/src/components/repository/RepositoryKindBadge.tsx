import { Layers, Radar, Warehouse } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import type { RepositoryKindDto } from "@/api/generated/RepositoryKindDto";

const KIND_META = {
  forge: { label: "Forge", icon: Warehouse },
  mirror: { label: "Mirror", icon: Radar },
  alloy: { label: "Alloy", icon: Layers },
} as const;

export function RepositoryKindBadge({ kind }: { kind: RepositoryKindDto }) {
  const meta = KIND_META[kind.type];
  const Icon = meta.icon;

  return (
    <Badge variant="secondary" className="gap-1.5 font-medium">
      <Icon className="size-3.5" />
      {meta.label}
    </Badge>
  );
}
