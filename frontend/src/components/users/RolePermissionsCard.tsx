import { Check, Minus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { roleLabel } from "@/auth/roles";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const ROLES = ["reader", "developer", "admin"] as const;

type RoleKey = (typeof ROLES)[number];

interface PermissionRow {
  readonly labelKey: string;
  readonly allowed: Readonly<Record<RoleKey, boolean>>;
}

const PERMISSIONS: readonly PermissionRow[] = [
  { labelKey: "permissions.view", allowed: { reader: true, developer: true, admin: true } },
  { labelKey: "permissions.download", allowed: { reader: true, developer: true, admin: true } },
  { labelKey: "permissions.search", allowed: { reader: true, developer: true, admin: true } },
  { labelKey: "permissions.tokens", allowed: { reader: true, developer: true, admin: true } },
  { labelKey: "permissions.publish", allowed: { reader: false, developer: true, admin: true } },
  { labelKey: "permissions.repos", allowed: { reader: false, developer: true, admin: true } },
  { labelKey: "permissions.configure", allowed: { reader: false, developer: true, admin: true } },
  { labelKey: "permissions.gc", allowed: { reader: false, developer: true, admin: true } },
  { labelKey: "permissions.users", allowed: { reader: false, developer: false, admin: true } },
  { labelKey: "permissions.groups", allowed: { reader: false, developer: false, admin: true } },
  {
    labelKey: "permissions.resetPassword",
    allowed: { reader: false, developer: false, admin: true },
  },
];

export function RolePermissionsCard() {
  const { t } = useTranslation();
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("permissions.title")}</CardTitle>
        <CardDescription>{t("permissions.description")}</CardDescription>
      </CardHeader>
      <CardContent className="px-0 sm:px-6">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("common.action")}</TableHead>
              {ROLES.map((role) => (
                <TableHead key={role} className="text-center">
                  {roleLabel(role)}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {PERMISSIONS.map((row) => (
              <TableRow key={row.labelKey}>
                <TableCell className="whitespace-normal font-medium">{t(row.labelKey)}</TableCell>
                {ROLES.map((role) => (
                  <TableCell key={role} className="text-center">
                    {row.allowed[role] ? (
                      <Check
                        className="mx-auto size-4 text-emerald-600"
                        aria-label={t("permissions.allowed")}
                      />
                    ) : (
                      <Minus
                        className="mx-auto size-4 text-muted-foreground"
                        aria-label={t("permissions.denied")}
                      />
                    )}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}
