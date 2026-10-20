// Syntax highlighting with Shiki, loaded on demand: the core, the GitHub light/dark themes and
// each grammar are separate chunks that only pages showing code pay for. Tokens carry both
// theme colours as CSS variables (`--shiki-light`, `--shiki-dark`); code.css picks one by
// `prefers-color-scheme`, matching the rest of the app.
import type { HighlighterCore, ThemedToken } from 'shiki/core';

export type Token = { content: string; style?: Record<string, string> };
export type Lines = Token[][];

/** Files larger than this are shown plain; tokenising them would stall the page. */
export const MAX_BYTES = 1024 * 1024;
export const MAX_LINES = 5000;

const EXT: Record<string, string> = {
  rs: 'rust',
  go: 'go',
  ts: 'typescript',
  mts: 'typescript',
  cts: 'typescript',
  tsx: 'tsx',
  js: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  jsx: 'jsx',
  json: 'json',
  json5: 'json',
  jsonc: 'json',
  yaml: 'yaml',
  yml: 'yaml',
  toml: 'toml',
  md: 'markdown',
  markdown: 'markdown',
  mdx: 'markdown',
  sh: 'shellscript',
  bash: 'shellscript',
  zsh: 'shellscript',
  fish: 'shellscript',
  py: 'python',
  pyi: 'python',
  rb: 'ruby',
  rake: 'ruby',
  gemspec: 'ruby',
  java: 'java',
  kt: 'kotlin',
  kts: 'kotlin',
  swift: 'swift',
  c: 'c',
  h: 'c',
  cc: 'cpp',
  cpp: 'cpp',
  cxx: 'cpp',
  hpp: 'cpp',
  hh: 'cpp',
  cs: 'csharp',
  html: 'html',
  htm: 'html',
  vue: 'html',
  css: 'css',
  scss: 'scss',
  sql: 'sql',
  dockerfile: 'dockerfile',
  mk: 'makefile',
  ini: 'ini',
  cfg: 'ini',
  conf: 'ini',
  env: 'ini',
  properties: 'ini',
  xml: 'xml',
  svg: 'xml',
  plist: 'xml',
  csproj: 'xml',
  graphql: 'graphql',
  gql: 'graphql',
  proto: 'proto',
  tf: 'terraform',
  tfvars: 'terraform',
  hcl: 'hcl',
  nix: 'nix',
  lua: 'lua',
  dart: 'dart',
  php: 'php',
  diff: 'diff',
  patch: 'diff',
};

const NAMES: Record<string, string> = {
  dockerfile: 'dockerfile',
  containerfile: 'dockerfile',
  makefile: 'makefile',
  gnumakefile: 'makefile',
  'cargo.lock': 'toml',
  gemfile: 'ruby',
  rakefile: 'ruby',
  'go.mod': 'go',
  'go.sum': 'ini',
  '.gitignore': 'ini',
  '.gitattributes': 'ini',
  '.editorconfig': 'ini',
  '.bashrc': 'shellscript',
  '.zshrc': 'shellscript',
  '.env': 'ini',
};

/** Aliases people write in fenced code blocks, mapped to Shiki ids. */
const ALIASES: Record<string, string> = {
  rs: 'rust',
  golang: 'go',
  ts: 'typescript',
  js: 'javascript',
  yml: 'yaml',
  md: 'markdown',
  sh: 'shellscript',
  bash: 'shellscript',
  shell: 'shellscript',
  zsh: 'shellscript',
  console: 'shellscript',
  py: 'python',
  rb: 'ruby',
  kt: 'kotlin',
  'c++': 'cpp',
  cs: 'csharp',
  'c#': 'csharp',
  docker: 'dockerfile',
  make: 'makefile',
  tf: 'terraform',
  gql: 'graphql',
  protobuf: 'proto',
  jsonc: 'json',
  json5: 'json',
  htm: 'html',
  vue: 'html',
  text: '',
  txt: '',
  plain: '',
  plaintext: '',
};

/** Shiki language id for a file path, or null when the file should stay plain. */
export function langForPath(path: string): string | null {
  const name = path.split('/').pop()?.toLowerCase() ?? '';
  if (NAMES[name]) return NAMES[name];
  if (name.startsWith('dockerfile.') || name.endsWith('.dockerfile')) return 'dockerfile';
  const dot = name.lastIndexOf('.');
  if (dot < 0) return null;
  const ext = name.slice(dot + 1);
  return EXT[ext] ?? null;
}

/** Shiki language id for a fenced code block's info string, or null. */
export function langForAlias(alias: string): string | null {
  const a = alias.trim().toLowerCase();
  if (!a) return null;
  if (a in ALIASES) return ALIASES[a] || null;
  if (LANGS[a]) return a;
  return null;
}

// Every grammar is its own chunk. Vite needs literal import paths to split them.
const LANGS: Record<string, () => Promise<unknown>> = {
  rust: () => import('shiki/langs/rust.mjs'),
  go: () => import('shiki/langs/go.mjs'),
  typescript: () => import('shiki/langs/typescript.mjs'),
  tsx: () => import('shiki/langs/tsx.mjs'),
  javascript: () => import('shiki/langs/javascript.mjs'),
  jsx: () => import('shiki/langs/jsx.mjs'),
  json: () => import('shiki/langs/json.mjs'),
  yaml: () => import('shiki/langs/yaml.mjs'),
  toml: () => import('shiki/langs/toml.mjs'),
  markdown: () => import('shiki/langs/markdown.mjs'),
  shellscript: () => import('shiki/langs/shellscript.mjs'),
  python: () => import('shiki/langs/python.mjs'),
  ruby: () => import('shiki/langs/ruby.mjs'),
  java: () => import('shiki/langs/java.mjs'),
  kotlin: () => import('shiki/langs/kotlin.mjs'),
  swift: () => import('shiki/langs/swift.mjs'),
  c: () => import('shiki/langs/c.mjs'),
  cpp: () => import('shiki/langs/cpp.mjs'),
  csharp: () => import('shiki/langs/csharp.mjs'),
  html: () => import('shiki/langs/html.mjs'),
  css: () => import('shiki/langs/css.mjs'),
  scss: () => import('shiki/langs/scss.mjs'),
  sql: () => import('shiki/langs/sql.mjs'),
  dockerfile: () => import('shiki/langs/dockerfile.mjs'),
  makefile: () => import('shiki/langs/makefile.mjs'),
  ini: () => import('shiki/langs/ini.mjs'),
  xml: () => import('shiki/langs/xml.mjs'),
  graphql: () => import('shiki/langs/graphql.mjs'),
  proto: () => import('shiki/langs/proto.mjs'),
  terraform: () => import('shiki/langs/terraform.mjs'),
  hcl: () => import('shiki/langs/hcl.mjs'),
  nix: () => import('shiki/langs/nix.mjs'),
  lua: () => import('shiki/langs/lua.mjs'),
  dart: () => import('shiki/langs/dart.mjs'),
  php: () => import('shiki/langs/php.mjs'),
  diff: () => import('shiki/langs/diff.mjs'),
};

export const THEMES = { light: 'github-light', dark: 'github-dark' } as const;

let core: Promise<HighlighterCore> | null = null;
const loaded = new Set<string>();
const loading = new Map<string, Promise<void>>();

async function highlighter(): Promise<HighlighterCore> {
  if (!core) {
    core = (async () => {
      const [{ createHighlighterCore }, { createJavaScriptRegexEngine }, light, dark] = await Promise.all([
        import('shiki/core'),
        import('shiki/engine/javascript'),
        import('shiki/themes/github-light.mjs'),
        import('shiki/themes/github-dark.mjs'),
      ]);
      return createHighlighterCore({ themes: [light.default, dark.default], langs: [], engine: createJavaScriptRegexEngine({ forgiving: true }) });
    })();
  }
  return core;
}

async function ensureLang(lang: string): Promise<HighlighterCore | null> {
  const load = LANGS[lang];
  if (!load) return null;
  const h = await highlighter();
  if (!loaded.has(lang)) {
    if (!loading.has(lang)) {
      loading.set(
        lang,
        (async () => {
          const mod = (await load()) as { default: unknown };
          await h.loadLanguage(mod.default as never);
          loaded.add(lang);
        })(),
      );
    }
    await loading.get(lang);
  }
  return h;
}

function convert(rows: ThemedToken[][]): Lines {
  return rows.map((row) => row.map((t) => ({ content: t.content, style: t.htmlStyle as Record<string, string> | undefined })));
}

/**
 * Tokenise `text` line by line for `lang`. Resolves to null when the language is unknown
 * or the text exceeds the limits, so callers fall back to plain text without flashing.
 */
export async function tokenize(text: string, lang: string | null): Promise<Lines | null> {
  if (!lang || text.length > MAX_BYTES) return null;
  const h = await ensureLang(lang);
  if (!h) return null;
  const { tokens } = h.codeToTokens(text, { lang, themes: THEMES, defaultColor: false });
  return convert(tokens);
}

/** HTML for a fenced block (used by the markdown post-processor). */
export async function toHtml(code: string, lang: string): Promise<string | null> {
  const h = await ensureLang(lang);
  if (!h) return null;
  return h.codeToHtml(code, { lang, themes: THEMES, defaultColor: false });
}

/** Trailing "\n" handling: `codeToTokens` drops a final empty line that `split('\n')` keeps. */
export function lineTokens(lines: Lines | null, i: number): Token[] | undefined {
  return lines?.[i];
}
