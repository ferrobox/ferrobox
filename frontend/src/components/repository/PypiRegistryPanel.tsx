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

export function PypiRegistryPanel({
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
  const repositoryUrl = `${baseUrl}/pypi/${repositoryId}/`;
  const indexUrl = `${baseUrl}/pypi/${repositoryId}/simple/`;
  const isMirror = kind === "mirror";
  const isAlloy = kind === "alloy";
  const readOnly = isMirror || isAlloy;

  const introTitle = isAlloy ? "Alloy PyPI" : isMirror ? "Mirror PyPI" : "Registro PyPI";
  const introBody = isAlloy
    ? "Este Alloy agrega Forges y/o Mirrors PyPI en una sola URL. pip install resuelve contra los miembros, en orden. No acepta twine upload: publica en un Forge miembro."
    : isMirror
      ? "Este Mirror cachea paquetes del upstream la primera vez que pip install los resuelve o descarga. No acepta twine upload ni yank."
      : "Este repositorio implementa el protocolo de PyPI: twine upload y pip install funcionan de forma nativa.";

  const body = (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-semibold text-foreground">{introTitle}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{introBody}</p>
      </div>

      {readOnly ? (
        <div className="space-y-2">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            1. Instala un paquete
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
          <p className="text-xs text-muted-foreground">
            La barra final de <code className="font-mono">/simple/</code> es obligatoria. En HTTP
            local, <code className="font-mono">pip</code> usa{" "}
            <code className="font-mono">--trusted-host</code> y <code className="font-mono">uv</code>{" "}
            usa <code className="font-mono">--allow-insecure-host</code>. Ambos piden el índice JSON
            PEP 691.
          </p>
        </div>
      ) : (
        <>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              1. Configura twine
            </p>
            <CopyableCodeBlock
              code={`[distutils]
index-servers = ferrobox

[ferrobox]
repository = ${repositoryUrl}
username = __token__
password = fb_…`}
            />
            <p className="text-xs text-muted-foreground">
              Ponlo en <code className="font-mono">~/.pypirc</code>. El token se crea en Seguridad.
              Copia el UUID completo (8-4-4-4-12).
            </p>
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              2. Publica tu paquete
            </p>
            <CopyableCodeBlock code="python -m build && twine upload --repository ferrobox dist/*" />
          </div>
          <div className="space-y-2">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
              3. Instálalo en otro entorno
            </p>
            <CopyableCodeBlock
              code={`pip install demo-ferrobox-pypi --index-url ${indexUrl} --trusted-host 127.0.0.1`}
            />
            <CopyableCodeBlock
              code={`uv pip install demo-ferrobox-pypi --index-url ${indexUrl} --allow-insecure-host 127.0.0.1`}
            />
            <p className="text-xs text-muted-foreground">
              En HTTP local, <code className="font-mono">uv</code> usa{" "}
              <code className="font-mono">--allow-insecure-host</code> en lugar de{" "}
              <code className="font-mono">--trusted-host</code>. pip y uv piden el índice JSON PEP
              691.
            </p>
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
