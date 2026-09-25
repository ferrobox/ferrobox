import { useTranslation } from "react-i18next";

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
  dryRunLabel,
  appliedLabel,
  summary,
}: {
  preview: CleanupPreviewResponse;
  showRepository?: boolean;
  dryRunLabel?: string;
  appliedLabel?: string;
  summary: string;
}) {
  const { t } = useTranslation();
  const resolvedDryRun = dryRunLabel ?? t("common.simulate");
  const resolvedApplied = appliedLabel ?? t("common.apply");
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={preview.dry_run ? "outline" : "destructive"}>
          {preview.dry_run ? resolvedDryRun : resolvedApplied}
        </Badge>
        <p className="text-sm text-muted-foreground">{summary}</p>
      </div>
      {preview.items.length === 0 ? null : (
        <Table>
          <TableHeader>
            <TableRow>
              {showRepository ? <TableHead>{t("common.repository")}</TableHead> : null}
              <TableHead>{t("common.package")}</TableHead>
              <TableHead>{t("common.version")}</TableHead>
              <TableHead>{t("common.reason")}</TableHead>
              <TableHead className="text-right">{t("common.size")}</TableHead>
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
