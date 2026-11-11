import type { ReactNode } from "react";

type Props = {
  id?: string;
  eyebrow?: string;
  title: ReactNode;
  intro?: ReactNode;
  children: ReactNode;
  className?: string;
};

export function Section({ id, eyebrow, title, intro, children, className = "" }: Props) {
  return (
    <section id={id} className={`border-t border-line/60 py-20 sm:py-28 ${className}`} aria-labelledby={id ? `${id}-title` : undefined}>
      <div className="mx-auto max-w-6xl px-5 sm:px-8">
        <div className="reveal max-w-3xl">
          {eyebrow && <p className="mb-3 font-mono text-xs uppercase tracking-[0.2em] text-link">{eyebrow}</p>}
          <h2 id={id ? `${id}-title` : undefined} className="text-3xl font-semibold sm:text-4xl">{title}</h2>
          {intro && <div className="mt-4 text-lg leading-relaxed text-muted">{intro}</div>}
        </div>
        <div className="mt-12">{children}</div>
      </div>
    </section>
  );
}
