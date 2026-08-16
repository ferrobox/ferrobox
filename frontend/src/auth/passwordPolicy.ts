export const PASSWORD_POLICY_HINT =
  "Al menos 8 caracteres, con una minúscula, una mayúscula y un dígito.";

export function passwordMeetsPolicy(password: string): boolean {
  return (
    password.length >= 8 &&
    /[a-z]/.test(password) &&
    /[A-Z]/.test(password) &&
    /\d/.test(password)
  );
}
