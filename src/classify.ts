// Files whose diffs are real but never worth reading line by line.
const NOISE_PATTERNS: RegExp[] = [
  /(^|\/)(package-lock\.json|pnpm-lock\.yaml|yarn\.lock|npm-shrinkwrap\.json)$/,
  /(^|\/)(poetry\.lock|Pipfile\.lock|Cargo\.lock|composer\.lock|go\.sum|uv\.lock)$/,
  /(^|\/)(node_modules|vendor|dist|build|out|coverage|\.next|__pycache__)\//,
  /(^|\/)__snapshots__\//,
  /\.snap$/,
  /\.min\.(js|css|mjs)$/,
  /\.(map|lock)$/,
  /(^|\/)[^/]*\.generated\.[^/]+$/,
  /\.pb\.go$/,
];

const LANGUAGES: Record<string, string> = {
  ts: 'typescript', tsx: 'tsx', mts: 'typescript', cts: 'typescript',
  js: 'javascript', jsx: 'jsx', mjs: 'javascript', cjs: 'javascript',
  py: 'python', rb: 'ruby', go: 'go', rs: 'rust', java: 'java',
  kt: 'kotlin', swift: 'swift', c: 'c', h: 'c', cc: 'cpp',
  cpp: 'cpp', hpp: 'cpp', cs: 'csharp', php: 'php', sh: 'shell',
  bash: 'shell', zsh: 'shell', fish: 'shell', sql: 'sql',
  css: 'css', scss: 'scss', less: 'less', html: 'html',
  vue: 'vue', svelte: 'svelte', json: 'json', jsonc: 'json',
  yml: 'yaml', yaml: 'yaml', toml: 'toml', ini: 'ini',
  xml: 'xml', md: 'markdown', mdx: 'mdx', rst: 'rst',
  graphql: 'graphql', gql: 'graphql', proto: 'protobuf',
  tf: 'terraform', lua: 'lua', vim: 'vim',
};

export function isNoise(path: string): boolean {
  return NOISE_PATTERNS.some((pattern) => pattern.test(path));
}

export function languageOf(path: string): string | null {
  const name = path.split('/').pop() ?? '';
  if (name.toLowerCase().startsWith('dockerfile')) return 'dockerfile';
  if (!name.includes('.')) return null;
  const extension = name.split('.').pop()?.toLowerCase() ?? '';
  return LANGUAGES[extension] ?? null;
}
