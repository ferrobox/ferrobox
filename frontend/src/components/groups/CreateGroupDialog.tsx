import { type FormEvent, useState } from "react";
import { Loader2, UsersRound } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useCreateGroup } from "@/api/queries";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const GROUP_NAME_HINT =
  "Letras, dígitos, guiones y guiones bajos. Máximo 64 caracteres.";

function isValidGroupName(name: string): boolean {
  return /^[A-Za-z0-9_-]{1,64}$/.test(name);
}

export function CreateGroupDialog() {
  const createGroup = useCreateGroup();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!isValidGroupName(trimmed)) {
      toast.error(GROUP_NAME_HINT);
      return;
    }
    try {
      const created = await createGroup.mutateAsync({ name: trimmed });
      setName("");
      setOpen(false);
      toast.success("Grupo creado");
      navigate(`/groups/${created.id}`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo crear el grupo");
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          setName("");
        }
      }}
    >
      <DialogTrigger asChild>
        <Button>
          <UsersRound />
          Nuevo grupo
        </Button>
      </DialogTrigger>
      <DialogContent>
        <form onSubmit={(event) => void onSubmit(event)}>
          <DialogHeader>
            <DialogTitle>Crear grupo</DialogTitle>
            <DialogDescription>
              Los grupos agrupan usuarios y limitan qué repositorios pueden ver.
              Quien pertenezca a un grupo solo verá los repositorios asignados a
              sus grupos.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="group-name">Nombre</Label>
              <Input
                id="group-name"
                autoComplete="off"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="equipo-backend"
              />
              <p className="text-xs text-muted-foreground">{GROUP_NAME_HINT}</p>
            </div>
          </div>
          <DialogFooter>
            <Button type="submit" disabled={createGroup.isPending || name.trim().length === 0}>
              {createGroup.isPending ? <Loader2 className="animate-spin" /> : null}
              Crear
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
