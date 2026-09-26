import { useTranslation } from "react-i18next";

import { useSettings } from "@/api/queries";
import { CopyableCodeBlock } from "@/components/repository/CopyableCodeBlock";
import { Skeleton } from "@/components/ui/skeleton";

export function GoRegistryPanel({
  repositoryId,
  kind = "forge",
  framed = true,
}: {
  repositoryId: string;
  kind?: "forge" | "mirror" | "alloy";
  framed?: boolean;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError } = useSettings();

  if (isPending) {
    return <Skeleton className="h-40 w-full rounded-lg" />;
  }

  if (isError || !data) {
    return (
      <p className="text-sm text-muted-foreground">{t("registry.publicUrlFailed")}</p>
    );
  }

  const proxyUrl = `${data.public_base_url.replace(/\/$/, "")}/go/${repositoryId}`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.goAlloy")
    : isMirror
      ? t("registry.goMirror")
      : t("registry.goTitle");
  const introBody = isAlloy
    ? t("registry.goAlloyBody")
    : isMirror
      ? t("registry.goMirrorBody")
      : t("registry.go");

  const env = `export GOPROXY=${proxyUrl}
export GONOSUMDB=*`;
  const get = `GOPROXY=${proxyUrl} GONOSUMDB=* go get github.com/example/hello@v1.0.0`;
  const publish = `curl -T hello.zip -H "Authorization: Bearer fb_…" \\
  ${proxyUrl}/github.com/example/hello/@v/v1.0.0.zip`;

  return (
    <div className={framed ? "rounded-lg border border-border bg-card p-4" : undefined}>
      <h3 className="text-sm font-semibold">{introTitle}</h3>
      <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>

      <div className="mt-4 space-y-1">
        <p className="text-xs font-medium text-foreground">{t("registry.goStep1")}</p>
        <p className="text-xs text-muted-foreground">{t("registry.goStep1Hint")}</p>
        <CopyableCodeBlock code={proxyUrl} />
      </div>

      <div className="mt-4 space-y-1">
        <p className="text-xs font-medium text-foreground">{t("registry.goStepEnv")}</p>
        <CopyableCodeBlock code={env} />
      </div>

      {readOnly ? (
        <div className="mt-4 space-y-1">
          <p className="text-xs font-medium text-foreground">{t("registry.goStepResolve")}</p>
          <CopyableCodeBlock code={get} />
        </div>
      ) : (
        <>
          <div className="mt-4 space-y-1">
            <p className="text-xs font-medium text-foreground">{t("registry.goStep2")}</p>
            <p className="text-xs text-muted-foreground">{t("registry.goStep2Hint")}</p>
            <CopyableCodeBlock code={publish} />
          </div>
          <div className="mt-4 space-y-1">
            <p className="text-xs font-medium text-foreground">{t("registry.goStepResolve")}</p>
            <CopyableCodeBlock code={get} />
          </div>
        </>
      )}
    </div>
  );
}
