import { Check, Copy } from "lucide-react";
import { type MouseEvent, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

export function CopyableCodeBlock({ code }: { code: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  async function handleCopy(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    window.getSelection()?.removeAllRanges();
    await navigator.clipboard.writeText(code);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="flex items-start rounded-md bg-muted">
      <pre className="min-w-0 flex-1 overflow-x-auto px-4 py-3 font-mono text-xs break-all whitespace-pre-wrap text-foreground">
        {code}
      </pre>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="mt-1.5 mr-1.5 size-7 shrink-0"
        onMouseDown={(event) => event.preventDefault()}
        onClick={(event) => {
          void handleCopy(event);
        }}
        aria-label={t("registry.copyClipboard")}
      >
        {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </Button>
    </div>
  );
}
