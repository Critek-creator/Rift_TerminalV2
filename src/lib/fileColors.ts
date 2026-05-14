export type FileColorCategory = 'rust' | 'script' | 'frontend' | 'config' | 'docs' | 'default';

const EXT_MAP: Record<string, FileColorCategory> = {
  '.rs': 'rust',
  '.ts': 'script',
  '.js': 'script',
  '.mjs': 'script',
  '.cjs': 'script',
  '.tsx': 'script',
  '.jsx': 'script',
  '.svelte': 'frontend',
  '.html': 'frontend',
  '.css': 'frontend',
  '.scss': 'frontend',
  '.vue': 'frontend',
  '.json': 'config',
  '.toml': 'config',
  '.yaml': 'config',
  '.yml': 'config',
  '.xml': 'config',
  '.lock': 'config',
  '.ini': 'config',
  '.env': 'config',
  '.md': 'docs',
  '.txt': 'docs',
  '.sh': 'docs',
  '.ps1': 'docs',
  '.bat': 'docs',
  '.cmd': 'docs',
};

const CATEGORY_CSS: Record<FileColorCategory, string> = {
  rust: 'var(--term-red)',
  script: 'var(--term-blue)',
  frontend: 'var(--term-cyan)',
  config: 'var(--term-purple)',
  docs: 'var(--term-green)',
  default: 'var(--amber-dim)',
};

export function fileColor(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot < 0) return CATEGORY_CSS.default;
  const cat = EXT_MAP[name.slice(dot).toLowerCase()];
  return cat ? CATEGORY_CSS[cat] : CATEGORY_CSS.default;
}
