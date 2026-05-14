export type FileColorCategory = 'rust' | 'script' | 'frontend' | 'config' | 'docs' | 'media' | 'default';

const EXT_MAP: Record<string, FileColorCategory> = {
  '.rs': 'rust',
  '.ts': 'script',
  '.js': 'script',
  '.mjs': 'script',
  '.cjs': 'script',
  '.tsx': 'script',
  '.jsx': 'script',
  '.kt': 'script',
  '.java': 'script',
  '.py': 'script',
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
  '.gradle': 'config',
  '.kts': 'config',
  '.pro': 'config',
  '.properties': 'config',
  '.md': 'docs',
  '.txt': 'docs',
  '.sh': 'docs',
  '.ps1': 'docs',
  '.bat': 'docs',
  '.cmd': 'docs',
  '.png': 'media',
  '.jpg': 'media',
  '.jpeg': 'media',
  '.gif': 'media',
  '.svg': 'media',
  '.ico': 'media',
  '.webp': 'media',
  '.bmp': 'media',
  '.mp3': 'media',
  '.wav': 'media',
  '.ogg': 'media',
  '.mp4': 'media',
  '.ttf': 'media',
  '.otf': 'media',
  '.woff': 'media',
  '.woff2': 'media',
};

const CATEGORY_CSS: Record<FileColorCategory, string> = {
  rust: 'var(--term-red)',
  script: 'var(--term-blue)',
  frontend: 'var(--term-cyan)',
  config: 'var(--term-purple)',
  docs: 'var(--term-green)',
  media: 'var(--term-white)',
  default: 'var(--amber-dim)',
};

export function fileColor(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot < 0) return CATEGORY_CSS.default;
  const cat = EXT_MAP[name.slice(dot).toLowerCase()];
  return cat ? CATEGORY_CSS[cat] : CATEGORY_CSS.default;
}
