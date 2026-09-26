import { useTranslation } from "react-i18next";

import { useSettings } from "@/api/queries";
import { CopyableCodeBlock } from "@/components/repository/CopyableCodeBlock";
import { Skeleton } from "@/components/ui/skeleton";

export function ConanRegistryPanel({
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

  const remoteUrl = `${data.public_base_url.replace(/\/$/, "")}/conan/${repositoryId}`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.conanAlloy")
    : isMirror
      ? t("registry.conanMirror")
      : t("registry.conanTitle");
  const introBody = isAlloy
    ? t("registry.conanAlloyBody")
    : isMirror
      ? t("registry.conanMirrorBody")
      : t("registry.conan");

  const remote = `conan remote add ferrobox ${remoteUrl}\nconan remote update ferrobox --url=${remoteUrl}`;
  const login = `conan remote login ferrobox __token__`;
  const upload = `conan new cmake_lib -d name=hello -d version=0.1\nconan create .\nconan upload "hello/0.1" -r ferrobox -c`;
  const install = isMirror
    ? `conan install --requires=zlib/1.3.1 -r ferrobox`
    : `conan install --requires=hello/0.1 -r ferrobox`;

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("registry.conanStep1")}
        </p>
        <CopyableCodeBlock code={remote} />
        <p className="text-xs text-muted-foreground">
          {t("registry.conanStep1Hint", { url: remoteUrl })}
        </p>
      </div>

      {readOnly ? null : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.conanStep2")}
          </p>
          <CopyableCodeBlock code={login} />
          <p className="text-xs text-muted-foreground">{t("registry.conanStep2Hint")}</p>
        </div>
      )}

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {readOnly ? t("registry.conanStepInstall") : t("registry.conanStepPublish")}
        </p>
        <CopyableCodeBlock code={readOnly ? install : `${upload}\n${install}`} />
      </div>
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
