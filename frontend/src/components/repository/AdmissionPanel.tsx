import { type FormEvent, useState } from "react";
import type { TFunction } from "i18next";
import { AlertCircle, FlaskConical, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { AdmissionEffectDto } from "@/api/generated/AdmissionEffectDto";
import type { AdmissionEventResponse } from "@/api/generated/AdmissionEventResponse";
import type { AdmissionPolicyRequest } from "@/api/generated/AdmissionPolicyRequest";
import type { AdmissionPolicyResponse } from "@/api/generated/AdmissionPolicyResponse";
import type { AdmissionPredicateDto } from "@/api/generated/AdmissionPredicateDto";
import type { AdmissionPreviewResponse } from "@/api/generated/AdmissionPreviewResponse";
import type { PackageEcosystemDto } from "@/api/generated/PackageEcosystemDto";
import {
  useAdmissionEvents,
  useAdmissionPolicy,
  useDryRunAdmission,
  useSaveAdmissionPolicy,
} from "@/api/queries";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const EMPTY_POLICY: AdmissionPolicyResponse = {
  enabled: false,
  when: "pull",
  predicate: "not_signed",
  effect: "deny",
  public_keys_pem: "",
  require_signed: false,
  require_verified: false,
  min_finding: null,
  forbidden_licenses: [],
  profile: null,
};

const COPYLEFT_LICENSES = [
  "GPL-2.0",
  "GPL-2.0-only",
  "GPL-2.0-or-later",
  "GPL-3.0",
  "GPL-3.0-only",
  "GPL-3.0-or-later",
  "AGPL-3.0",
  "AGPL-3.0-only",
  "AGPL-3.0-or-later",
  "SSPL-1.0",
];

type FindingLevel = "" | "medium" | "high" | "critical";

function payloadFromFields(
  enabled: boolean,
  effect: AdmissionEffectDto,
  requireSigned: boolean,
  requireVerified: boolean,
  minFinding: FindingLevel,
  forbiddenLicenses: string[],
  publicKeysPem: string,
  profile: string | null,
): AdmissionPolicyRequest {
  const predicate: AdmissionPredicateDto =
    requireVerified && !requireSigned ? "not_verified" : "not_signed";
  return {
    enabled,
    when: "pull",
    predicate,
    effect,
    public_keys_pem: publicKeysPem,
    require_signed: requireSigned,
    require_verified: requireVerified,
    min_finding: minFinding.length > 0 ? minFinding : null,
    forbidden_licenses: forbiddenLicenses,
    profile,
  };
}

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

export function AdmissionPanel({
  repositoryId,
  canWrite,
  ecosystem,
}: {
  repositoryId: string;
  canWrite: boolean;
  ecosystem: PackageEcosystemDto;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError, error } = useAdmissionPolicy(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("admission.title")}</CardTitle>
        <CardDescription>{t("admission.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError && error.message ? (
          isMissingMigration(error.message) ? (
            <MigrationAlert />
          ) : (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertTitle>{t("admission.loadFailed")}</AlertTitle>
              <AlertDescription>
                {t("admission.loadFailedHint", { message: error.message })}
              </AlertDescription>
            </Alert>
          )
        ) : null}
        {isPending ? null : (
          <AdmissionForm
            repositoryId={repositoryId}
            canWrite={canWrite}
            ecosystem={ecosystem}
            policy={data ?? EMPTY_POLICY}
          />
        )}
        <AdmissionEventLog repositoryId={repositoryId} />
      </CardContent>
    </Card>
  );
}

function MigrationAlert() {
  const { t } = useTranslation();
  return (
    <Alert variant="destructive">
      <AlertCircle />
      <AlertTitle>{t("retention.migrationTitle")}</AlertTitle>
      <AlertDescription className="gap-2">
        <p>{t("retention.migrationBody")}</p>
        <pre className="mt-1 w-full overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs text-foreground">
          sqlx migrate run
        </pre>
        <p>{t("admission.migrationHint")}</p>
      </AlertDescription>
    </Alert>
  );
}

function AdmissionForm({
  repositoryId,
  canWrite,
  ecosystem,
  policy,
}: {
  repositoryId: string;
  canWrite: boolean;
  ecosystem: PackageEcosystemDto;
  policy: AdmissionPolicyResponse;
}) {
  const { t } = useTranslation();
  const savePolicy = useSaveAdmissionPolicy(repositoryId);
  const dryRun = useDryRunAdmission(repositoryId);
  const showSignature = ecosystem === "oci" || ecosystem === "helm";
  const [enabled, setEnabled] = useState(policy.enabled);
  const [effect, setEffect] = useState<AdmissionEffectDto>(policy.effect);
  const [requireSigned, setRequireSigned] = useState(policy.require_signed);
  const [requireVerified, setRequireVerified] = useState(policy.require_verified);
  const [minFinding, setMinFinding] = useState<FindingLevel>(
    policy.min_finding === "medium" ||
      policy.min_finding === "high" ||
      policy.min_finding === "critical"
      ? policy.min_finding
      : "",
  );
  const [licensesText, setLicensesText] = useState(policy.forbidden_licenses.join("\n"));
  const [profile, setProfile] = useState(policy.profile ?? "");
  const [publicKeysPem, setPublicKeysPem] = useState(policy.public_keys_pem);
  const [preview, setPreview] = useState<AdmissionPreviewResponse | null>(null);
  const [schemaError, setSchemaError] = useState(false);
  const missingKeys = requireVerified && publicKeysPem.trim() === "";

  function licenses(): string[] {
    return licensesText
      .split(/\r?\n/)
      .map((item) => item.trim())
      .filter((item) => item.length > 0);
  }

  function currentPayload(): AdmissionPolicyRequest {
    return payloadFromFields(
      enabled,
      effect,
      showSignature && requireSigned,
      showSignature && requireVerified,
      minFinding,
      licenses(),
      publicKeysPem,
      profile.length > 0 ? profile : null,
    );
  }

  function rememberSchemaError(err: unknown) {
    const message = err instanceof ApiError ? err.message : "";
    if (isMissingMigration(message)) {
      setSchemaError(true);
    }
  }

  function touch() {
    setPreview(null);
    setProfile("");
  }

  function applyProfile(next: string) {
    setProfile(next);
    setPreview(null);
    if (next === "openchain_security") {
      setEffect("warn");
      setRequireSigned(false);
      setRequireVerified(false);
      setMinFinding("medium");
      setLicensesText("");
    } else if (next === "copyleft_restrict") {
      setEffect("deny");
      setRequireSigned(false);
      setRequireVerified(false);
      setMinFinding("");
      setLicensesText(COPYLEFT_LICENSES.join("\n"));
    } else if (next === "critical_only") {
      setEffect("deny");
      setRequireSigned(false);
      setRequireVerified(false);
      setMinFinding("critical");
      setLicensesText("");
    }
  }

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      await savePolicy.mutateAsync(currentPayload());
      setSchemaError(false);
      toast.success(enabled ? t("admission.savedOn") : t("admission.savedOff"));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : t("admission.saveFailed"));
    }
  }

  async function onSimulate() {
    try {
      const result = await dryRun.mutateAsync(currentPayload());
      setPreview(result);
      setSchemaError(false);
      toast.success(previewMessage(result, t));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : t("admission.simulateFailed"));
    }
  }

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      {schemaError ? <MigrationAlert /> : null}

      <ol className="space-y-4 text-sm">
        <li className="space-y-3">
          <p className="font-medium text-foreground">{t("admission.step1")}</p>
          <label className="flex cursor-pointer items-start gap-2">
            <input
              type="checkbox"
              className="mt-1 size-4 accent-primary"
              checked={enabled}
              onChange={(event) => {
                setEnabled(event.target.checked);
                setPreview(null);
              }}
              disabled={!canWrite}
            />
            <span>
              <span className="font-medium">{t("admission.enable")}</span>
              <span className="block text-xs text-muted-foreground">
                {t("admission.enableHint")}
              </span>
            </span>
          </label>
          <div className="space-y-2">
            <Label htmlFor="admission-profile">{t("admission.profile")}</Label>
            <Select
              value={profile.length > 0 ? profile : "none"}
              onValueChange={(value) => applyProfile(value === "none" ? "" : value)}
              disabled={!canWrite}
            >
              <SelectTrigger id="admission-profile">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">{t("admission.profileNone")}</SelectItem>
                <SelectItem value="openchain_security">{t("admission.profileOpenchain")}</SelectItem>
                <SelectItem value="copyleft_restrict">{t("admission.profileCopyleft")}</SelectItem>
                <SelectItem value="critical_only">{t("admission.profileCritical")}</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {profile === "openchain_security"
                ? t("admission.profileOpenchainHint")
                : profile === "copyleft_restrict"
                  ? t("admission.profileCopyleftHint")
                  : profile === "critical_only"
                    ? t("admission.profileCriticalHint")
                    : t("admission.findingHint")}
            </p>
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>{t("admission.when")}</Label>
              <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm">
                {t("admission.whenPull")}
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="admission-effect">{t("admission.then")}</Label>
              <Select
                value={effect}
                onValueChange={(value) => {
                  setEffect(value as AdmissionEffectDto);
                  touch();
                }}
                disabled={!canWrite}
              >
                <SelectTrigger id="admission-effect">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="deny">{t("admission.denyPull")}</SelectItem>
                  <SelectItem value="warn">{t("admission.warnAllow")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          {showSignature ? (
            <div className="space-y-2">
              <p className="text-sm font-medium">{t("admission.if")}</p>
              <label className="flex cursor-pointer items-start gap-2">
                <input
                  type="checkbox"
                  className="mt-1 size-4 accent-primary"
                  checked={requireSigned}
                  onChange={(event) => {
                    setRequireSigned(event.target.checked);
                    touch();
                  }}
                  disabled={!canWrite}
                />
                <span>{t("admission.notSigned")}</span>
              </label>
              <label className="flex cursor-pointer items-start gap-2">
                <input
                  type="checkbox"
                  className="mt-1 size-4 accent-primary"
                  checked={requireVerified}
                  onChange={(event) => {
                    setRequireVerified(event.target.checked);
                    touch();
                  }}
                  disabled={!canWrite}
                />
                <span>{t("admission.notVerified")}</span>
              </label>
            </div>
          ) : null}
          {showSignature && requireVerified ? (
            <div className="space-y-2">
              <Label htmlFor="admission-keys">{t("admission.keys")}</Label>
              <textarea
                id="admission-keys"
                className="min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
                value={publicKeysPem}
                onChange={(event) => {
                  setPublicKeysPem(event.target.value);
                  setPreview(null);
                }}
                disabled={!canWrite}
                placeholder={"-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----"}
                spellCheck={false}
              />
              <p className="text-xs text-muted-foreground">{t("admission.keysHint")}</p>
              {missingKeys ? (
                <Alert>
                  <AlertCircle />
                  <AlertTitle>{t("admission.noKeysTitle")}</AlertTitle>
                  <AlertDescription>{t("admission.noKeysBody")}</AlertDescription>
                </Alert>
              ) : null}
            </div>
          ) : null}
          <div className="space-y-2">
            <Label htmlFor="admission-finding">{t("admission.findings")}</Label>
            <Select
              value={minFinding.length > 0 ? minFinding : "off"}
              onValueChange={(value) => {
                setMinFinding(value === "off" ? "" : (value as FindingLevel));
                touch();
              }}
              disabled={!canWrite}
            >
              <SelectTrigger id="admission-finding">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="off">{t("admission.findingOff")}</SelectItem>
                <SelectItem value="medium">{t("admission.findingMedium")}</SelectItem>
                <SelectItem value="high">{t("admission.findingHigh")}</SelectItem>
                <SelectItem value="critical">{t("admission.findingCritical")}</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">{t("admission.findingHint")}</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="admission-licenses">{t("admission.licenses")}</Label>
            <textarea
              id="admission-licenses"
              className="min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
              value={licensesText}
              onChange={(event) => {
                setLicensesText(event.target.value);
                touch();
              }}
              disabled={!canWrite}
              placeholder={t("admission.licensesPlaceholder")}
              spellCheck={false}
            />
            <p className="text-xs text-muted-foreground">{t("admission.licensesHint")}</p>
          </div>
        </li>
        {canWrite ? (
          <li className="space-y-3">
            <p className="font-medium text-foreground">{t("admission.step2")}</p>
            <p className="text-muted-foreground">{t("admission.step2Hint")}</p>
            <Button
              type="button"
              variant="outline"
              disabled={dryRun.isPending}
              onClick={() => void onSimulate()}
            >
              <FlaskConical />
              {dryRun.isPending ? t("common.simulating") : t("common.simulate")}
            </Button>
          </li>
        ) : (
          <p className="text-muted-foreground">{t("admission.readOnly")}</p>
        )}
      </ol>

      {preview ? <AdmissionPreviewTable preview={preview} /> : null}

      {canWrite ? (
        <div className="space-y-3">
          <p className="text-sm font-medium text-foreground">{t("admission.step3")}</p>
          <p className="text-sm text-muted-foreground">{t("admission.step3Hint")}</p>
          <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
            <Save />
            {savePolicy.isPending ? t("common.saving") : t("common.save")}
          </Button>
        </div>
      ) : null}
    </form>
  );
}

function previewMessage(preview: AdmissionPreviewResponse, t: TFunction): string {
  const allowed =
    preview.allowed === 1
      ? t("admission.oneWouldPass")
      : t("admission.manyWouldPass", { count: preview.allowed });
  if (preview.matches.length === 0) {
    return t("admission.noneWouldFire", { allowed });
  }
  if (preview.matches.length === 1) {
    return t("admission.oneWouldFire", { allowed });
  }
  return t("admission.manyWouldFire", { count: preview.matches.length, allowed });
}

function formatEventTime(value: string): string {
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Date(parsed).toLocaleString();
}

function AdmissionEventLog({ repositoryId }: { repositoryId: string }) {
  const { t } = useTranslation();
  const { data, isPending, isError, error } = useAdmissionEvents(repositoryId, true);

  return (
    <div className="space-y-3 border-t border-border pt-5">
      <div>
        <p className="text-sm font-medium text-foreground">{t("admission.eventsTitle")}</p>
        <p className="text-sm text-muted-foreground">{t("admission.eventsHint")}</p>
      </div>
      {isPending ? <Skeleton className="h-16 w-full rounded-md" /> : null}
      {isError && error.message ? (
        <p className="text-sm text-muted-foreground">{error.message}</p>
      ) : null}
      {data && data.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("admission.noEvents")}</p>
      ) : null}
      {data && data.length > 0 ? <AdmissionEventTable events={data} /> : null}
    </div>
  );
}

function AdmissionEventTable({ events }: { events: AdmissionEventResponse[] }) {
  const { t } = useTranslation();
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>{t("common.when")}</TableHead>
          <TableHead>{t("admission.image")}</TableHead>
          <TableHead>{t("admission.tag")}</TableHead>
          <TableHead>{t("admission.effect")}</TableHead>
          <TableHead>{t("common.reason")}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {events.map((event) => (
          <TableRow key={event.id}>
            <TableCell className="text-xs text-muted-foreground">
              {formatEventTime(event.created_at)}
            </TableCell>
            <TableCell className="font-mono text-xs">{event.name}</TableCell>
            <TableCell className="font-mono text-xs">{event.reference}</TableCell>
            <TableCell>
              <Badge variant={event.effect === "deny" ? "destructive" : "outline"}>
                {event.effect === "deny" ? t("admission.denied") : t("admission.warned")}
              </Badge>
            </TableCell>
            <TableCell>{event.reason}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function AdmissionPreviewTable({ preview }: { preview: AdmissionPreviewResponse }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">{t("admission.simulation")}</Badge>
        <p className="text-sm text-muted-foreground">{previewMessage(preview, t)}</p>
      </div>
      {preview.matches.length === 0 ? null : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("admission.image")}</TableHead>
              <TableHead>{t("admission.tag")}</TableHead>
              <TableHead>{t("admission.effect")}</TableHead>
              <TableHead>{t("common.reason")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {preview.matches.map((item) => (
              <TableRow key={`${item.name}:${item.version}:${item.effect}`}>
                <TableCell className="font-mono text-xs">{item.name}</TableCell>
                <TableCell className="font-mono text-xs">{item.version}</TableCell>
                <TableCell>
                  {item.effect === "deny" ? t("admission.denyAction") : t("admission.warnAction")}
                </TableCell>
                <TableCell>{item.reason}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
