import { type ChangeEvent, useRef } from "react";
import { Loader2, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { usePublishArtifact } from "@/api/queries";
import { Button } from "@/components/ui/button";

export function UploadArtifactButton({ repositoryId }: { repositoryId: string }) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLInputElement>(null);
  const mutation = usePublishArtifact(repositoryId);

  function handleChange(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) {
      return;
    }

    mutation.mutate(file, {
      onSuccess: () => toast.success(t("upload.published", { name: file.name })),
      onError: (error) => {
        const message = error instanceof ApiError ? error.message : error.message;
        toast.error(t("upload.failed", { name: file.name }), { description: message });
      },
    });
  }

  return (
    <>
      <input ref={inputRef} type="file" className="hidden" onChange={handleChange} />
      <Button onClick={() => inputRef.current?.click()} disabled={mutation.isPending}>
        {mutation.isPending ? <Loader2 className="animate-spin" /> : <Upload />}
        {t("upload.action")}
      </Button>
    </>
  );
}
