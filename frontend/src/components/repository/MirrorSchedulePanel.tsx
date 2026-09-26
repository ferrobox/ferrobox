import { type FormEvent, useEffect, useState } from "react";
import { Loader2, Save, Timer } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useSetMirrorSchedule } from "@/api/queries";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

function formatLastRun(value: string | null, locale: string, neverLabel: string): string {
  if (!value) {
    return neverLabel;
  }
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Intl.DateTimeFormat(locale.startsWith("es") ? "es" : "en", {
    dateStyle: "short",
    timeStyle: "medium",
  }).format(parsed);
}

export function MirrorSchedulePanel({
  repositoryId,
  intervalHours,
  lastPrefetchAt,
  canWrite,
}: {
  repositoryId: string;
  intervalHours: number | null;
  lastPrefetchAt: string | null;
  canWrite: boolean;
}) {
  const { t, i18n } = useTranslation();
  const save = useSetMirrorSchedule(repositoryId);
  const [hours, setHours] = useState(intervalHours == null ? "" : String(intervalHours));

  useEffect(() => {
    setHours(intervalHours == null ? "" : String(intervalHours));
  }, [intervalHours]);

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = hours.trim();
    let interval: number | undefined;
    if (trimmed.length > 0) {
      const parsed = Number.parseInt(trimmed, 10);
      if (!Number.isFinite(parsed) || parsed < 0 || parsed > 168) {
        toast.error(t("repositories.scheduleHoursHint"));
        return;
      }
      interval = parsed;
    }
    try {
      await save.mutateAsync({ prefetch_interval_hours: interval });
      toast.success(t("repositories.scheduleSaved"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("repositories.scheduleFailed"));
    }
  }

  return (
    <form
      onSubmit={(event) => void onSubmit(event)}
      className="flex flex-wrap items-end gap-3 rounded-lg border border-border bg-card px-3 py-2"
    >
      <Timer className="mb-1 size-4 text-muted-foreground" />
      <div className="min-w-[8rem] space-y-1">
        <Label htmlFor="mirror-schedule-hours">{t("repositories.schedule")}</Label>
        <Input
          id="mirror-schedule-hours"
          type="number"
          min={0}
          max={168}
          placeholder={t("repositories.scheduleOff")}
          value={hours}
          onChange={(event) => setHours(event.target.value)}
          disabled={!canWrite || save.isPending}
        />
      </div>
      <p className="max-w-xs text-xs text-muted-foreground">{t("repositories.scheduleHint")}</p>
      <p className="text-xs text-muted-foreground">
        {t("repositories.scheduleLastRun")}:{" "}
        {formatLastRun(lastPrefetchAt, i18n.language, t("repositories.scheduleNever"))}
      </p>
      {canWrite ? (
        <Button type="submit" size="sm" disabled={save.isPending}>
          {save.isPending ? <Loader2 className="animate-spin" /> : <Save />}
          {t("repositories.scheduleSave")}
        </Button>
      ) : null}
    </form>
  );
}
