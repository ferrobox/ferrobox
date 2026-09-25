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

export function MavenRegistryPanel({
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

  const repositoryUrl = `${data.public_base_url.replace(/\/$/, "")}/maven/${repositoryId}`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy
    ? t("registry.mavenAlloy")
    : isMirror
      ? t("registry.mavenMirror")
      : t("registry.mavenTitle");
  const introBody = isAlloy
    ? t("registry.mavenAlloyBody")
    : isMirror
      ? t("registry.mavenMirrorBody")
      : t("registry.maven");

  const settingsXml = `<settings>
  <servers>
    <server>
      <id>ferrobox</id>
      <username>__token__</username>
      <password>fb_…</password>
    </server>
  </servers>
</settings>`;

  const pomXml = `<distributionManagement>
  <repository>
    <id>ferrobox</id>
    <url>${repositoryUrl}</url>
  </repository>
  <snapshotRepository>
    <id>ferrobox</id>
    <url>${repositoryUrl}</url>
  </snapshotRepository>
</distributionManagement>`;

  const resolve = `mvn dependency:get -DremoteRepositories=ferrobox::::${repositoryUrl} -Dartifact=org.example:hello:1.0.0`;
  const deploy = `mvn deploy`;
  const gradle = `repositories {
    maven { url = uri("${repositoryUrl}") }
}`;

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("registry.mavenStep1")}
        </p>
        <CopyableCodeBlock code={repositoryUrl} />
        <p className="text-xs text-muted-foreground">{t("registry.mavenStep1Hint")}</p>
      </div>

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("registry.mavenStepResolve")}
          </p>
          <CopyableCodeBlock code={resolve} />
          <CopyableCodeBlock code={gradle} />
        </div>
      ) : (
        <>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.mavenStep2")}
            </p>
            <CopyableCodeBlock code={settingsXml} />
            <p className="text-xs text-muted-foreground">{t("registry.mavenStep2Hint")}</p>
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.mavenStep3")}
            </p>
            <CopyableCodeBlock code={pomXml} />
            <CopyableCodeBlock code={deploy} />
            <p className="text-xs text-muted-foreground">{t("registry.mavenStep3Hint")}</p>
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {t("registry.mavenStepGradle")}
            </p>
            <CopyableCodeBlock code={gradle} />
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
