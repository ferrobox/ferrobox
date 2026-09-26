import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useSettings } from "@/api/queries";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

function CopyableCodeBlock({ code }: { code: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    await navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="group relative">
      <pre className="overflow-x-auto rounded-md bg-muted px-4 py-3 font-mono text-xs text-foreground">
        {code}
      </pre>
      <Button
        variant="ghost"
        size="icon"
        className="absolute top-1.5 right-1.5 size-7 opacity-0 transition-opacity group-hover:opacity-100"
        onClick={() => void handleCopy()}
        aria-label={t("registry.copyClipboard")}
      >
        {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </Button>
    </div>
  );
}

export function HelmRegistryPanel({
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
  const chartRef = `oci://${registryHost}/${repositoryId}/demo`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;
  const plainHttp = baseUrl.startsWith("http://");

  const introTitle = isAlloy
    ? t("registry.helmAlloy")
    : isMirror
      ? t("registry.helmMirror")
      : t("registry.helmTitle");
  const introBody = isAlloy
    ? t("registry.helmAlloyBody")
    : isMirror
      ? t("registry.helmMirrorBody")
      : t("registry.helm");

  const login = `echo 'fb_…' | helm registry login ${registryHost} -u __token__ --password-stdin`;
  const pushPull = plainHttp
    ? `helm package ./demo\nhelm push demo-0.1.0.tgz ${chartRef.replace(/\/demo$/, "")} --plain-http\nhelm pull ${chartRef} --version 0.1.0 --plain-http`
    : `helm package ./demo\nhelm push demo-0.1.0.tgz ${chartRef.replace(/\/demo$/, "")}\nhelm pull ${chartRef} --version 0.1.0`;
  const pullOnly = plainHttp
    ? `helm pull ${chartRef} --version 0.1.0 --plain-http`
    : `helm pull ${chartRef} --version 0.1.0`;
  const hubCommand = `helm pull oci://${registryHost}/${repositoryId}/bitnami/nginx --version 18.0.0${plainHttp ? " --plain-http" : ""}`;

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      {readOnly ? null : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.helmStep1")}
          </p>
          <CopyableCodeBlock code={login} />
          <p className="text-xs text-muted-foreground">
            {t("registry.helmStep1Hint", { host: registryHost })}
          </p>
        </div>
      )}
      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {readOnly ? t("registry.helmStepPull") : t("registry.helmStepPush")}
        </p>
        <CopyableCodeBlock code={readOnly ? pullOnly : pushPull} />
        <p className="text-xs text-muted-foreground">
          {t("registry.helmStepHint")}
          {isMirror ? ` ${t("registry.helmMirrorHub", { command: hubCommand })}` : null}
        </p>
      </div>
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
