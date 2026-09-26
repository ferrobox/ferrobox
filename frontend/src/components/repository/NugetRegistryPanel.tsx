import { useTranslation } from "react-i18next";

import { useSettings } from "@/api/queries";
import { CopyableCodeBlock } from "@/components/repository/CopyableCodeBlock";
import { Skeleton } from "@/components/ui/skeleton";

export function NugetRegistryPanel({
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

  const sourceUrl = `${data.public_base_url.replace(/\/$/, "")}/nuget/${repositoryId}/v3/index.json`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.nugetAlloy")
    : isMirror
      ? t("registry.nugetMirror")
      : t("registry.nugetTitle");
  const introBody = isAlloy
    ? t("registry.nugetAlloyBody")
    : isMirror
      ? t("registry.nugetMirrorBody")
      : t("registry.nuget");

  const addSource = `dotnet nuget add source ${sourceUrl} -n ferrobox -u __token__ -p fb_… --store-password-in-clear-text`;
  const push = `dotnet pack
dotnet nuget push ./bin/Release/*.nupkg --source ferrobox --api-key fb_…`;
  const restore = `dotnet add package Hello.World --source ferrobox
dotnet restore`;

  const nugetConfig = `<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <add key="ferrobox" value="${sourceUrl}" />
  </packageSources>
  <packageSourceCredentials>
    <ferrobox>
      <add key="Username" value="__token__" />
      <add key="ClearTextPassword" value="fb_…" />
    </ferrobox>
  </packageSourceCredentials>
</configuration>`;

  return (
    <div className={framed ? "rounded-lg border border-border bg-card p-4" : undefined}>
      <h3 className="text-sm font-semibold">{introTitle}</h3>
      <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>

      <div className="mt-4 space-y-1">
        <p className="text-xs font-medium text-foreground">{t("registry.nugetStep1")}</p>
        <p className="text-xs text-muted-foreground">{t("registry.nugetStep1Hint")}</p>
        <CopyableCodeBlock code={sourceUrl} />
      </div>

      {readOnly ? (
        <div className="mt-4 space-y-1">
          <p className="text-xs font-medium text-foreground">{t("registry.nugetStepResolve")}</p>
          <CopyableCodeBlock code={restore} />
        </div>
      ) : (
        <>
          <div className="mt-4 space-y-1">
            <p className="text-xs font-medium text-foreground">{t("registry.nugetStep2")}</p>
            <p className="text-xs text-muted-foreground">{t("registry.nugetStep2Hint")}</p>
            <CopyableCodeBlock code={addSource} />
          </div>
          <div className="mt-4 space-y-1">
            <p className="text-xs font-medium text-foreground">{t("registry.nugetStep3")}</p>
            <p className="text-xs text-muted-foreground">{t("registry.nugetStep3Hint")}</p>
            <CopyableCodeBlock code={push} />
          </div>
          <div className="mt-4 space-y-1">
            <p className="text-xs font-medium text-foreground">{t("registry.nugetStepConfig")}</p>
            <CopyableCodeBlock code={nugetConfig} />
          </div>
        </>
      )}
    </div>
  );
}
