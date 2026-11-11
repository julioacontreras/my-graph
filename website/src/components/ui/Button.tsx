import type { ReactNode } from "react";

type Props = {
  href: string;
  children: ReactNode;
  variant?: "primary" | "secondary";
  external?: boolean;
};

export function Button({ href, children, variant = "primary", external }: Props) {
  const base =
    "inline-flex min-h-11 items-center justify-center gap-2 rounded-lg px-5 text-sm font-semibold transition-colors";
  const style =
    variant === "primary"
      ? "bg-primary text-white hover:bg-primary-hover"
      : "border border-line bg-surface text-fg hover:border-muted hover:bg-elevated";
  return (
    <a
      href={href}
      className={`${base} ${style}`}
      {...(external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
    >
      {children}
    </a>
  );
}
