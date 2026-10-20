// Server-rendered, sanitised HTML from comrak. The server is the only place markdown is
// turned into HTML, so the client never needs a parser or a sanitiser. Fenced code blocks
// (`<pre lang="rust"><code>` from comrak, or `<code class="language-rust">`) are highlighted
// after render; the original <pre> is kept so its styling and any copy button survive.
import { useEffect, useRef } from 'react';
import { langForAlias, toHtml } from '../lib/highlight';

export function Markdown({ html }: { html: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    let alive = true;
    const blocks = Array.from(root.querySelectorAll<HTMLPreElement>('pre'));
    for (const pre of blocks) {
      if (pre.dataset.highlighted) continue;
      const code = pre.querySelector('code');
      if (!code) continue;
      const cls = Array.from(code.classList).find((c) => c.startsWith('language-'));
      const lang = langForAlias(pre.getAttribute('lang') ?? cls?.slice('language-'.length) ?? '');
      if (!lang) continue;
      pre.dataset.highlighted = 'pending';
      const source = code.textContent ?? '';
      toHtml(source.replace(/\n$/, ''), lang)
        .then((out) => {
          if (!alive || !out) return;
          const doc = new DOMParser().parseFromString(out, 'text/html');
          const inner = doc.querySelector('pre > code');
          if (!inner) return;
          code.innerHTML = inner.innerHTML;
          code.classList.add('shiki');
          pre.dataset.highlighted = 'done';
        })
        .catch(() => {
          if (alive) delete pre.dataset.highlighted;
        });
    }
    return () => {
      alive = false;
    };
  }, [html]);
  return <div ref={ref} className="markdown-body" dangerouslySetInnerHTML={{ __html: html }} />;
}
