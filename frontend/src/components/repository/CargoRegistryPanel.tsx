import { useTranslation } from "react-i18next";

import { useCargoRegistryConfig } from "@/api/queries";
import { CopyableCodeBlock } from "@/components/repository/CopyableCodeBlock";
import { Skeleton } from "@/components/ui/skeleton";

export function CargoRegistryPanel({
  repositoryId,
  kind = "forge",
  framed = true,
}: {
  repositoryId: string;
  kind?: "forge" | "mirror" | "alloy";
  framed?: boolean;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError } = useCargoRegistryConfig(repositoryId, true);

  if (isPending) {
    return <Skeleton className="h-40 w-full rounded-lg" />;
  }

  if (isError || !data) {
    return (
      <p className="text-sm text-muted-foreground">
        {t("registry.cargoFailed")}
      </p>
    );
  }

  const indexUrl = `sparse+${data.api}/`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.cargoAlloy")
    : isMirror
      ? t("registry.cargoMirror")
      : t("registry.cargoTitle");
  const introBody = isAlloy
    ? t("registry.cargoAlloyBody")
    : isMirror
      ? t("registry.cargoMirrorBody")
      : t("registry.cargo");

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">
          {introTitle}
        </h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("registry.cargoStep1")}
        </p>
        <CopyableCodeBlock
          code={`[registries.ferrobox]\nindex = "${indexUrl}"`}
        />
        <p className="text-xs text-muted-foreground">{t("registry.cargoStep1Hint")}</p>
      </div>

      {!readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.cargoStep2Token")}
          </p>
          <CopyableCodeBlock
            code={`[registries.ferrobox]\ntoken = "fb_…"  # ${t("security.createToken")}`}
          />
          <p className="text-xs text-muted-foreground">{t("registry.cargoStep2TokenHint")}</p>
        </div>
      ) : null}

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.cargoStep2Use")}
          </p>
          <CopyableCodeBlock code="cargo add serde --registry ferrobox" />
          {isAlloy ? (
            <p className="text-xs text-muted-foreground">{t("registry.cargoAlloyPublishHint")}</p>
          ) : null}
        </div>
      ) : (
        <>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.cargoStep3")}
            </p>
            <CopyableCodeBlock code="cargo publish --registry ferrobox" />
          </div>

          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.cargoStep4")}
            </p>
            <CopyableCodeBlock code="cargo add mi-crate --registry ferrobox" />
          </div>

          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.cargoStep5")}
            </p>
            <CopyableCodeBlock code="cargo yank --vers 0.1.0 --registry ferrobox" />
          </div>
        </>
      )}
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
