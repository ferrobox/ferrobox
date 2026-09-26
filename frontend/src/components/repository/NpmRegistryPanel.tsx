import { useTranslation } from "react-i18next";

import { useSettings } from "@/api/queries";
import { CopyableCodeBlock } from "@/components/repository/CopyableCodeBlock";
import { Skeleton } from "@/components/ui/skeleton";

export function NpmRegistryPanel({
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

  const registryUrl = `${data.public_base_url.replace(/\/$/, "")}/npm/${repositoryId}/`;
  const authHost = registryUrl.replace(/^https?:\/\//, "");
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.npmAlloy")
    : isMirror
      ? t("registry.npmMirror")
      : t("registry.npmTitle");
  const introBody = isAlloy
    ? t("registry.npmAlloyBody")
    : isMirror
      ? t("registry.npmMirrorBody")
      : t("registry.npm");

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("registry.npmStep1")}
        </p>
        <CopyableCodeBlock
          code={`registry=${registryUrl}\n//${authHost}:_authToken=fb_…`}
        />
        <p className="text-xs text-muted-foreground">{t("registry.npmStep1Hint")}</p>
      </div>

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.npmStepInstall")}
          </p>
          <CopyableCodeBlock code="npm install demo-pkg" />
        </div>
      ) : (
        <>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.npmStepPublish")}
            </p>
            <CopyableCodeBlock code="npm publish" />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.npmStepInstallOther")}
            </p>
            <CopyableCodeBlock code="npm install demo-pkg" />
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
