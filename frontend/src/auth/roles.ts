import type { RoleDto } from "@/api/generated/RoleDto";

export function canManageUsers(role: RoleDto | undefined | null): boolean {
  return role === "admin";
}

export function canWriteArtifacts(role: RoleDto | undefined | null): boolean {
  return role === "admin" || role === "developer";
}

export function roleLabel(role: RoleDto): string {
  switch (role) {
    case "admin":
      return "Admin";
    case "developer":
      return "Developer";
    case "reader":
      return "Reader";
  }
}
