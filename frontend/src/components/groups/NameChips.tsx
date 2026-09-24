import { Badge } from "@/components/ui/badge";

interface NameChipsProps {
  readonly names: readonly string[];
  readonly empty: string;
}

export function NameChips({ names, empty }: NameChipsProps) {
  if (names.length === 0) {
    return <span className="text-muted-foreground">{empty}</span>;
  }

  return (
    <div className="flex flex-wrap gap-1">
      {names.map((name) => (
        <Badge key={name} variant="secondary">
          {name}
        </Badge>
      ))}
    </div>
  );
}
