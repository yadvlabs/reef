// React glue for the highlighter: a hook that tokenises text in the background and a
// component that renders one line's tokens (or plain text until they arrive).
import { useEffect, useState } from 'react';
import { tokenize, type Lines, type Token } from './index';

/** Tokens for `text` in `lang`, or null while loading / when highlighting does not apply. */
export function useHighlight(text: string, lang: string | null, enabled = true): Lines | null {
  const [lines, setLines] = useState<Lines | null>(null);
  useEffect(() => {
    let alive = true;
    setLines(null);
    if (!enabled || !lang) return;
    tokenize(text, lang)
      .then((t) => alive && setLines(t))
      .catch(() => alive && setLines(null));
    return () => {
      alive = false;
    };
  }, [text, lang, enabled]);
  return lines;
}

/** One line of code: highlighted spans when tokens exist, otherwise the raw text. */
export function CodeText({ tokens, text }: { tokens?: Token[]; text: string }) {
  if (!tokens || tokens.length === 0) return <>{text}</>;
  return (
    <>
      {tokens.map((t, i) =>
        t.style ? (
          <span key={i} className="tok" style={t.style as React.CSSProperties}>
            {t.content}
          </span>
        ) : (
          <span key={i}>{t.content}</span>
        ),
      )}
    </>
  );
}
