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
};

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

function payloadFromFields(
  enabled: boolean,
  effect: AdmissionEffectDto,
  predicate: AdmissionPredicateDto,
  publicKeysPem: string,
): AdmissionPolicyRequest {
  return {
    enabled,
    when: "pull",
    predicate,
    effect,
    public_keys_pem: publicKeysPem,
  };
}

export function AdmissionPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
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
  policy,
}: {
  repositoryId: string;
  canWrite: boolean;
  policy: AdmissionPolicyResponse;
}) {
  const { t } = useTranslation();
  const savePolicy = useSaveAdmissionPolicy(repositoryId);
  const dryRun = useDryRunAdmission(repositoryId);
  const [enabled, setEnabled] = useState(policy.enabled);
  const [effect, setEffect] = useState<AdmissionEffectDto>(policy.effect);
  const [predicate, setPredicate] = useState<AdmissionPredicateDto>(policy.predicate);
  const [publicKeysPem, setPublicKeysPem] = useState(policy.public_keys_pem);
  const [preview, setPreview] = useState<AdmissionPreviewResponse | null>(null);
  const [schemaError, setSchemaError] = useState(false);
  const missingKeys = predicate === "not_verified" && publicKeysPem.trim() === "";

  function rememberSchemaError(err: unknown) {
    const message = err instanceof ApiError ? err.message : "";
    if (isMissingMigration(message)) {
      setSchemaError(true);
    }
  }

  function onFieldsChange(
    nextEnabled: boolean,
    nextEffect: AdmissionEffectDto,
    nextPredicate: AdmissionPredicateDto,
    nextKeys: string,
  ) {
    setEnabled(nextEnabled);
    setEffect(nextEffect);
    setPredicate(nextPredicate);
    setPublicKeysPem(nextKeys);
    setPreview(null);
  }

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      await savePolicy.mutateAsync(
        payloadFromFields(enabled, effect, predicate, publicKeysPem),
      );
      setSchemaError(false);
      toast.success(
        enabled
          ? predicate === "not_verified"
            ? t("admission.savedUnsigned")
            : t("admission.savedUnsignedSig")
          : t("admission.savedOff"),
      );
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : t("admission.saveFailed"));
    }
  }

  async function onSimulate() {
    try {
      const result = await dryRun.mutateAsync(
        payloadFromFields(enabled, effect, predicate, publicKeysPem),
      );
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
              onChange={(event) =>
                onFieldsChange(event.target.checked, effect, predicate, publicKeysPem)
              }
              disabled={!canWrite}
            />
            <span>
              <span className="font-medium">{t("admission.enable")}</span>
              <span className="block text-xs text-muted-foreground">
                {t("admission.enableHint")}
              </span>
            </span>
          </label>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>{t("admission.when")}</Label>
              <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm">
                {t("admission.whenPull")}
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="admission-predicate">{t("admission.if")}</Label>
              <Select
                value={predicate}
                onValueChange={(value) =>
                  onFieldsChange(
                    enabled,
                    effect,
                    value as AdmissionPredicateDto,
                    publicKeysPem,
                  )
                }
                disabled={!canWrite}
              >
                <SelectTrigger id="admission-predicate">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="not_signed">{t("admission.notSigned")}</SelectItem>
                  <SelectItem value="not_verified">{t("admission.notVerified")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <div className="space-y-2">
            <Label htmlFor="admission-keys">{t("admission.keys")}</Label>
            <textarea
              id="admission-keys"
              className="min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
              value={publicKeysPem}
              onChange={(event) =>
                onFieldsChange(enabled, effect, predicate, event.target.value)
              }
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
          <div className="space-y-2">
            <Label htmlFor="admission-effect">{t("admission.then")}</Label>
            <Select
              value={effect}
              onValueChange={(value) =>
                onFieldsChange(enabled, value as AdmissionEffectDto, predicate, publicKeysPem)
              }
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
