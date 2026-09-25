import { Box, Coffee, Container, FileArchive, Package, Ship, Sparkle, Wrench } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import { cn } from "@/lib/utils";

const ECOSYSTEM_META: Record<
  PackageEcosystemDto,
  { label: string; icon: typeof Box; className: string }
> = {
  generic: {
    label: "Generic",
    icon: Box,
    className: "bg-slate-100 text-slate-700 dark:bg-slate-500/15 dark:text-slate-300",
  },
  cargo: {
    label: "Cargo",
    icon: Package,
    className: "bg-orange-100 text-orange-700 dark:bg-orange-500/15 dark:text-orange-300",
  },
  npm: {
    label: "npm",
    icon: Sparkle,
    className: "bg-red-100 text-red-700 dark:bg-red-500/15 dark:text-red-300",
  },
  pypi: {
    label: "PyPI",
    icon: FileArchive,
    className: "bg-blue-100 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300",
  },
  oci: {
    label: "OCI",
    icon: Container,
    className: "bg-cyan-100 text-cyan-700 dark:bg-cyan-500/15 dark:text-cyan-300",
  },
  helm: {
    label: "Helm",
    icon: Ship,
    className: "bg-indigo-100 text-indigo-700 dark:bg-indigo-500/15 dark:text-indigo-300",
  },
  conan: {
    label: "Conan",
    icon: Wrench,
    className: "bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300",
  },
  maven: {
    label: "Maven",
    icon: Coffee,
    className: "bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300",
  },
};

export function ecosystemMeta(ecosystem: PackageEcosystemDto) {
  return ECOSYSTEM_META[ecosystem];
}

export function EcosystemBadge({ ecosystem }: { ecosystem: PackageEcosystemDto }) {
  const meta = ECOSYSTEM_META[ecosystem];
  const Icon = meta.icon;

  return (
    <Badge variant="outline" className={cn("gap-1.5 border-transparent font-medium", meta.className)}>
      <Icon className="size-3.5" />
      {meta.label}
    </Badge>
  );
}

export const ECOSYSTEM_OPTIONS: readonly { value: PackageEcosystemDto; label: string }[] = (
  Object.keys(ECOSYSTEM_META) as PackageEcosystemDto[]
).map((value) => ({ value, label: ECOSYSTEM_META[value].label }));
