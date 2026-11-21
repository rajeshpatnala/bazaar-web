import type { ButtonHTMLAttributes } from "react";

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "ghost";
};

/** Always type="button" unless overridden, so it never submits a form by accident. */
export function Button({ variant = "primary", type = "button", className, ...rest }: ButtonProps) {
  return <button type={type} className={`ui-btn ui-btn-${variant} ${className ?? ""}`.trim()} {...rest} />;
}
