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

export function PypiRegistryPanel({
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
  const repositoryUrl = `${baseUrl}/pypi/${repositoryId}/`;
  const indexUrl = `${baseUrl}/pypi/${repositoryId}/simple/`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.pypiAlloy")
    : isMirror
      ? t("registry.pypiMirror")
      : t("registry.pypiTitle");
  const introBody = isAlloy
    ? t("registry.pypiAlloyBody")
    : isMirror
      ? t("registry.pypiMirrorBody")
      : t("registry.pypi");

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.pypiStepInstall")}
          </p>
          <CopyableCodeBlock
            code={
              isMirror
                ? `pip install requests --index-url ${indexUrl} --trusted-host 127.0.0.1`
                : `pip install demo-ferrobox-pypi --index-url ${indexUrl} --trusted-host 127.0.0.1`
            }
          />
          <CopyableCodeBlock
            code={
              isMirror
                ? `uv pip install requests --index-url ${indexUrl} --allow-insecure-host 127.0.0.1`
                : `uv pip install demo-ferrobox-pypi --index-url ${indexUrl} --allow-insecure-host 127.0.0.1`
            }
          />
          <p className="text-xs text-muted-foreground">{t("registry.pypiStepInstallHint")}</p>
        </div>
      ) : (
        <>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.pypiStepTwine")}
            </p>
            <CopyableCodeBlock
              code={`[distutils]
index-servers = ferrobox

[ferrobox]
repository = ${repositoryUrl}
username = __token__
password = fb_…`}
            />
            <p className="text-xs text-muted-foreground">{t("registry.pypiStepTwineHint")}</p>
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.pypiStepPublish")}
            </p>
            <CopyableCodeBlock code="python -m build && twine upload --repository ferrobox dist/*" />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.pypiStepInstallOther")}
            </p>
            <CopyableCodeBlock
              code={`pip install demo-ferrobox-pypi --index-url ${indexUrl} --trusted-host 127.0.0.1`}
            />
            <CopyableCodeBlock
              code={`uv pip install demo-ferrobox-pypi --index-url ${indexUrl} --allow-insecure-host 127.0.0.1`}
            />
            <p className="text-xs text-muted-foreground">{t("registry.pypiStepInstallOtherHint")}</p>
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
