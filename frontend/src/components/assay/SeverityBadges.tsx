import type { AssayCountsDto } from "@/api/generated/AssayCountsDto";
import type { AssaySeverityDto } from "@/api/generated/AssaySeverityDto";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

const SEVERITY_CLASS: Record<AssaySeverityDto, string> = {
  critical: "border-red-500/50 bg-red-500/15 text-red-300",
  high: "border-orange-500/50 bg-orange-500/15 text-orange-300",
  medium: "border-amber-500/50 bg-amber-500/15 text-amber-300",
  low: "border-sky-500/50 bg-sky-500/15 text-sky-300",
  unknown: "border-border bg-muted text-muted-foreground",
};

const SEVERITY_LABEL: Record<AssaySeverityDto, string> = {
  critical: "Crítica",
  high: "Alta",
  medium: "Media",
  low: "Baja",
  unknown: "Sin puntuación",
};

export function SeverityBadge({ severity }: { severity: AssaySeverityDto }) {
  return (
    <Badge variant="outline" className={SEVERITY_CLASS[severity]}>
      {SEVERITY_LABEL[severity]}
    </Badge>
  );
}

export function AssayCountPills({
  counts,
  compact = false,
}: {
  counts: AssayCountsDto;
  compact?: boolean;
}) {
  const items: Array<[AssaySeverityDto, number]> = compact
    ? [
        ["critical", counts.critical],
        ["high", counts.high],
      ]
    : [
        ["critical", counts.critical],
        ["high", counts.high],
        ["medium", counts.medium],
        ["low", counts.low],
        ["unknown", counts.unknown],
      ];
  const visible = items.filter(([, count]) => count > 0);
  if (visible.length === 0) {
    if (compact) {
      return null;
    }
    return <span className="text-xs text-muted-foreground">Sin hallazgos</span>;
  }
  return (
    <span className="flex flex-wrap items-center gap-1">
      {visible.map(([severity, count]) => (
        <Badge
          key={severity}
          variant="outline"
          className={cn("font-mono", compact && "px-1.5 py-0 text-[10px]", SEVERITY_CLASS[severity])}
        >
          {count}
          {compact ? "" : ` ${SEVERITY_LABEL[severity].toLowerCase()}`}
        </Badge>
      ))}
    </span>
  );
}
