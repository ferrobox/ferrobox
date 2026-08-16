import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatBytes } from "@/lib/format";

export function CleanupPreviewTable({
  preview,
  showRepository,
  dryRunLabel = "Simulación",
  appliedLabel = "Aplicado",
  summary,
}: {
  preview: CleanupPreviewResponse;
  showRepository?: boolean;
  dryRunLabel?: string;
  appliedLabel?: string;
  summary: string;
}) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={preview.dry_run ? "outline" : "destructive"}>
          {preview.dry_run ? dryRunLabel : appliedLabel}
        </Badge>
        <p className="text-sm text-muted-foreground">{summary}</p>
      </div>
      {preview.items.length === 0 ? null : (
        <Table>
          <TableHeader>
            <TableRow>
              {showRepository ? <TableHead>Repositorio</TableHead> : null}
              <TableHead>Paquete</TableHead>
              <TableHead>Versión</TableHead>
              <TableHead>Motivo</TableHead>
              <TableHead className="text-right">Tamaño</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {preview.items.map((item) => (
              <TableRow
                key={`${item.repository}:${item.name}:${item.version}:${item.reason}`}
              >
                {showRepository ? (
                  <TableCell className="font-mono text-xs">{item.repository}</TableCell>
                ) : null}
                <TableCell className="font-mono text-xs">
                  {item.name.length > 0 ? item.name : "—"}
                </TableCell>
                <TableCell className="font-mono text-xs">{item.version}</TableCell>
                <TableCell>{item.reason}</TableCell>
                <TableCell className="text-right">{formatBytes(item.size_bytes)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
