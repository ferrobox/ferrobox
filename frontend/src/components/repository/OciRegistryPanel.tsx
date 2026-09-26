import { useTranslation } from "react-i18next";

import { useSettings } from "@/api/queries";
import { CopyableCodeBlock } from "@/components/repository/CopyableCodeBlock";
import { Skeleton } from "@/components/ui/skeleton";

export function OciRegistryPanel({
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

  const baseUrl = data.public_base_url.replace(/\/$/, "");
  const registryHost = baseUrl.replace(/^https?:\/\//, "");
  const imageRef = `${registryHost}/${repositoryId}/demo:latest`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.ociAlloy")
    : isMirror
      ? t("registry.ociMirror")
      : t("registry.ociTitle");
  const introBody = isAlloy
    ? t("registry.ociAlloyBody")
    : isMirror
      ? t("registry.ociMirrorBody")
      : t("registry.oci");

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      {readOnly ? null : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.ociStep1")}
          </p>
          <CopyableCodeBlock
            code={`echo 'fb_…' | docker login ${registryHost} -u __token__ --password-stdin`}
          />
          <p className="text-xs text-muted-foreground">
            {t("registry.ociStep1Hint", { host: registryHost })}
          </p>
        </div>
      )}
      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {readOnly ? t("registry.ociStepPull") : t("registry.ociStepPush")}
        </p>
        <CopyableCodeBlock
          code={
            readOnly
              ? `docker pull ${imageRef}`
              : `docker tag alpine:latest ${imageRef}\ndocker push ${imageRef}\ndocker pull ${imageRef}`
          }
        />
        <p className="text-xs text-muted-foreground">
          {t("registry.ociStepPushHint")}
          {isMirror ? ` ${t("registry.ociMirrorHub")}` : null}
        </p>
      </div>
      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.ociStepVerify")}
          </p>
          <CopyableCodeBlock code={`cosign verify --key cosign.pub ${imageRef}`} />
          <p className="text-xs text-muted-foreground">{t("registry.ociStepVerifyHint")}</p>
        </div>
      ) : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.ociStepSign")}
          </p>
          <CopyableCodeBlock
            code={`cosign generate-key-pair
cosign sign --key cosign.key ${imageRef}
cosign verify --key cosign.pub ${imageRef}`}
          />
          <p className="text-xs text-muted-foreground">{t("registry.ociStepSignHint")}</p>
        </div>
      )}
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
