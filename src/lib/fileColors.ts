type FileColorCategory =
  | 'rust' | 'typescript' | 'javascript' | 'python' | 'go' | 'java'
  | 'shell' | 'cpp' | 'svelte' | 'html' | 'css' | 'json' | 'yaml'
  | 'toml' | 'markdown' | 'media' | 'binary' | 'default';

const EXT_MAP: Record<string, FileColorCategory> = {
  '.rs': 'rust',
  '.ts': 'typescript', '.tsx': 'typescript',
  '.js': 'javascript', '.mjs': 'javascript', '.cjs': 'javascript', '.jsx': 'javascript',
  '.py': 'python', '.pyw': 'python', '.pyi': 'python',
  '.go': 'go',
  '.java': 'java', '.kt': 'java', '.kts': 'java',
  '.sh': 'shell', '.bash': 'shell', '.zsh': 'shell', '.ps1': 'shell', '.bat': 'shell', '.cmd': 'shell',
  '.c': 'cpp', '.cpp': 'cpp', '.h': 'cpp', '.hpp': 'cpp',
  '.rb': 'python', '.r': 'python',
  '.cs': 'java', '.swift': 'java', '.dart': 'java',
  '.lua': 'javascript', '.php': 'javascript',
  '.svelte': 'svelte',
  '.html': 'html', '.vue': 'html', '.astro': 'html', '.hbs': 'html',
  '.css': 'css', '.scss': 'css', '.sass': 'css', '.less': 'css',
  '.json': 'json', '.lock': 'json',
  '.yaml': 'yaml', '.yml': 'yaml',
  '.toml': 'toml', '.ini': 'toml', '.env': 'toml', '.cfg': 'toml', '.conf': 'toml',
  '.editorconfig': 'toml', '.xml': 'toml', '.gradle': 'toml', '.pro': 'toml', '.properties': 'toml',
  '.md': 'markdown', '.txt': 'markdown', '.rst': 'markdown', '.adoc': 'markdown', '.log': 'markdown',
  '.png': 'media', '.jpg': 'media', '.jpeg': 'media', '.gif': 'media', '.svg': 'media',
  '.ico': 'media', '.webp': 'media', '.bmp': 'media',
  '.mp3': 'media', '.wav': 'media', '.ogg': 'media', '.flac': 'media',
  '.mp4': 'media', '.avi': 'media', '.mov': 'media',
  '.ttf': 'media', '.otf': 'media', '.woff': 'media', '.woff2': 'media',
  '.bin': 'binary', '.jar': 'binary', '.aar': 'binary', '.apk': 'binary',
  '.dex': 'binary', '.so': 'binary', '.dll': 'binary', '.exe': 'binary',
  '.wasm': 'binary', '.zip': 'binary', '.tar': 'binary', '.gz': 'binary',
};

const CATEGORY_CSS: Record<FileColorCategory, string> = {
  rust:       'var(--term-red)',
  typescript: '#3178C6',
  javascript: '#F0DB4F',
  python:     '#4B8BBE',
  go:         '#00ADD8',
  java:       '#ED8B00',
  shell:      'var(--term-green)',
  cpp:        '#659AD2',
  svelte:     '#FF3E00',
  html:       '#E34C26',
  css:        '#563D7C',
  json:       'var(--term-purple)',
  yaml:       '#CB171E',
  toml:       'var(--amber-dim)',
  markdown:   'var(--term-green)',
  media:      'var(--term-cyan)',
  binary:     'var(--amber-faint)',
  default:    'var(--amber-faint)',
};

export function fileColor(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot < 0) return CATEGORY_CSS.default;
  const cat = EXT_MAP[name.slice(dot).toLowerCase()];
  return cat ? CATEGORY_CSS[cat] : CATEGORY_CSS.default;
}
