import { type ChangeEvent, useRef } from "react";
import { Loader2, Upload } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { usePublishArtifact } from "@/api/queries";
import { Button } from "@/components/ui/button";

export function UploadArtifactButton({ repositoryId }: { repositoryId: string }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const mutation = usePublishArtifact(repositoryId);

  function handleChange(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) {
      return;
    }

    mutation.mutate(file, {
      onSuccess: () => toast.success(`«${file.name}» publicado correctamente.`),
      onError: (error) => {
        const message = error instanceof ApiError ? error.message : error.message;
        toast.error(`No se pudo publicar «${file.name}»`, { description: message });
      },
    });
  }

  return (
    <>
      <input ref={inputRef} type="file" className="hidden" onChange={handleChange} />
      <Button onClick={() => inputRef.current?.click()} disabled={mutation.isPending}>
        {mutation.isPending ? <Loader2 className="animate-spin" /> : <Upload />}
        Subir artefacto
      </Button>
    </>
  );
}
