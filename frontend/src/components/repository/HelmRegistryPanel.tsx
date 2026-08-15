import { Check, Copy } from "lucide-react";
import { useState } from "react";

import { useSettings } from "@/api/queries";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

function CopyableCodeBlock({ code }: { code: string }) {
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
        aria-label="Copiar al portapapeles"
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
  const { data, isPending, isError } = useSettings();

  if (isPending) {
    return <Skeleton className="h-40 w-full rounded-lg" />;
  }

  if (isError || !data) {
    return (
      <p className="text-sm text-muted-foreground">
        No se pudo obtener la URL pública de la instancia.
      </p>
    );
  }

  const baseUrl = data.public_base_url.replace(/\/$/, "");
  const registryHost = baseUrl.replace(/^https?:\/\//, "");
  const chartRef = `oci://${registryHost}/${repositoryId}/demo`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;
  const plainHttp = baseUrl.startsWith("http://");

  const introTitle = isAlloy ? "Alloy Helm" : isMirror ? "Mirror Helm" : "Registro Helm";
  const introBody = isAlloy
    ? "Este Alloy agrega Forges y/o Mirrors Helm en una sola URL. helm pull e helm install resuelven contra los miembros, en orden. No acepta helm push: publica en un Forge miembro."
    : isMirror
      ? "Este Mirror cachea charts OCI del upstream la primera vez que helm pull los resuelve. No acepta helm push ni yank."
      : "Los charts se publican como artefactos OCI (Distribution Spec v2). helm push y helm pull hablan con /v2/ en la raíz del host.";

  const login = `echo 'fb_…' | helm registry login ${registryHost} -u __token__ --password-stdin`;
  const pushPull = plainHttp
    ? `helm package ./demo\nhelm push demo-0.1.0.tgz ${chartRef.replace(/\/demo$/, "")} --plain-http\nhelm pull ${chartRef} --version 0.1.0 --plain-http`
    : `helm package ./demo\nhelm push demo-0.1.0.tgz ${chartRef.replace(/\/demo$/, "")}\nhelm pull ${chartRef} --version 0.1.0`;
  const pullOnly = plainHttp
    ? `helm pull ${chartRef} --version 0.1.0 --plain-http`
    : `helm pull ${chartRef} --version 0.1.0`;

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      {readOnly ? null : (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            1. Login (token de Seguridad)
          </p>
          <CopyableCodeBlock code={login} />
          <p className="text-xs text-muted-foreground">
            Copia el UUID completo (8-4-4-4-12). En HTTP local,{" "}
            <code className="font-mono">helm push</code> / <code className="font-mono">helm pull</code>{" "}
            necesitan <code className="font-mono">--plain-http</code>. Si un push anterior falló con
            401, <code className="font-mono">helm logout {registryHost}</code> y vuelve a entrar:
            Helm puede haber cacheado un token anónimo del ping a{" "}
            <code className="font-mono">/v2/</code>.
          </p>
        </div>
      )}
      <div className="space-y-2">
        <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {readOnly ? "1. Instala (pull)" : "2. Empaqueta, publica y tira"}
        </p>
        <CopyableCodeBlock code={readOnly ? pullOnly : pushPull} />
        <p className="text-xs text-muted-foreground">
          El chart queda en <code className="font-mono">oci://&lt;host&gt;/&lt;UUID&gt;/&lt;nombre&gt;</code>
          . El nombre sale de <code className="font-mono">Chart.yaml</code>. Los nombres con barra
          (por ejemplo <code className="font-mono">bitnami/nginx</code>) también funcionan.
          {isMirror ? (
            <>
              {" "}
              Con upstream Docker Hub:{" "}
              <code className="font-mono">
                helm pull oci://{registryHost}/{repositoryId}/bitnami/nginx --version 18.0.0
                {plainHttp ? " --plain-http" : ""}
              </code>
              .
            </>
          ) : null}
        </p>
      </div>
    </div>
  );

  if (!framed) {
    return body;
  }

  return <div className="rounded-lg border border-border p-5">{body}</div>;
}
