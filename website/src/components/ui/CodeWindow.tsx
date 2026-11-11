import { snippets } from "../../data/snippets.generated";
import { CopyButton } from "./CopyButton";

type Props = {
  id: keyof typeof snippets;
  filename?: string;
  className?: string;
  copy?: boolean;
};

/** A dark code window with a file-name tab and a copy button. HTML comes from Shiki at build time. */
export function CodeWindow({ id, filename, className = "", copy = true }: Props) {
  const s = snippets[id];
  return (
    <div className={`overflow-hidden rounded-xl border border-line bg-surface ${className}`}>
      <div className="flex items-center justify-between border-b border-line bg-[#11182a] pl-4 pr-2 py-1.5">
        <div className="flex items-center gap-3">
          <span className="flex gap-1.5" aria-hidden="true">
            <span className="size-2.5 rounded-full bg-[#3b4660]" />
            <span className="size-2.5 rounded-full bg-[#3b4660]" />
            <span className="size-2.5 rounded-full bg-[#3b4660]" />
          </span>
          {filename && <span className="font-mono text-xs text-muted">{filename}</span>}
        </div>
        {copy && <CopyButton text={s.code} />}
      </div>
      <div className="code-html" dangerouslySetInnerHTML={{ __html: s.html }} />
    </div>
  );
}

/** A one-line shell command with a prompt and a copy button. */
export function CommandLine({ id }: { id: keyof typeof snippets }) {
  const s = snippets[id];
  return (
    <div className="flex items-center gap-2 rounded-lg border border-line bg-surface py-1.5 pl-4 pr-1.5">
      <span className="select-none font-mono text-sm text-violet" aria-hidden="true">$</span>
      <div className="code-html min-w-0 flex-1 [&_pre]:!p-0 [&_pre]:!py-1 [&_pre]:whitespace-pre" dangerouslySetInnerHTML={{ __html: s.html }} />
      <CopyButton text={s.code} />
    </div>
  );
}
