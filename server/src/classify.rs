//! Which files are worth reading line by line, and what to highlight them as.

/// Lockfiles, generated output and vendored trees. Their diffs are real, they
/// are just never read; the UI folds them and the line cap is tighter.
const NOISE_NAMES: [&str; 11] = [
  "package-lock.json",
  "pnpm-lock.yaml",
  "pnpm-lock.yml",
  "yarn.lock",
  "npm-shrinkwrap.json",
  "poetry.lock",
  "Pipfile.lock",
  "Cargo.lock",
  "composer.lock",
  "go.sum",
  "uv.lock",
];

const NOISE_DIRS: [&str; 9] = [
  "node_modules",
  "vendor",
  "dist",
  "build",
  "out",
  "coverage",
  ".next",
  "__pycache__",
  "__snapshots__",
];

const NOISE_SUFFIXES: [&str; 7] =
  [".snap", ".map", ".lock", ".pb.go", ".min.js", ".min.css", ".min.mjs"];

const LANGUAGES: [(&str, &str); 49] = [
  ("ts", "typescript"),
  ("tsx", "tsx"),
  ("mts", "typescript"),
  ("cts", "typescript"),
  ("js", "javascript"),
  ("jsx", "jsx"),
  ("mjs", "javascript"),
  ("cjs", "javascript"),
  ("py", "python"),
  ("rb", "ruby"),
  ("go", "go"),
  ("rs", "rust"),
  ("java", "java"),
  ("kt", "kotlin"),
  ("swift", "swift"),
  ("c", "c"),
  ("h", "c"),
  ("cc", "cpp"),
  ("cpp", "cpp"),
  ("hpp", "cpp"),
  ("cs", "csharp"),
  ("php", "php"),
  ("sh", "shell"),
  ("bash", "shell"),
  ("zsh", "shell"),
  ("fish", "shell"),
  ("sql", "sql"),
  ("css", "css"),
  ("scss", "scss"),
  ("less", "less"),
  ("html", "html"),
  ("vue", "vue"),
  ("svelte", "svelte"),
  ("json", "json"),
  ("jsonc", "json"),
  ("yml", "yaml"),
  ("yaml", "yaml"),
  ("toml", "toml"),
  ("ini", "ini"),
  ("xml", "xml"),
  ("md", "markdown"),
  ("mdx", "mdx"),
  ("rst", "rst"),
  ("graphql", "graphql"),
  ("gql", "graphql"),
  ("proto", "protobuf"),
  ("tf", "terraform"),
  ("lua", "lua"),
  ("vim", "vim"),
];

fn basename(path: &str) -> &str {
  path.rsplit('/').next().unwrap_or(path)
}

pub fn is_noise(path: &str) -> bool {
  let name = basename(path);
  if NOISE_NAMES.contains(&name) {
    return true;
  }
  // Every segment but the last, so `lib/dist-helper.ts` and a file *named*
  // `dist` stay ordinary source.
  if path.split('/').rev().skip(1).any(|segment| NOISE_DIRS.contains(&segment)) {
    return true;
  }
  NOISE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
    || name.split('.').nth_back(1) == Some("generated")
}

pub fn language_of(path: &str) -> Option<&'static str> {
  let name = basename(path);
  if name.to_ascii_lowercase().starts_with("dockerfile") {
    return Some("dockerfile");
  }
  if !name.contains('.') {
    return None;
  }
  let extension = name.rsplit('.').next()?.to_ascii_lowercase();
  LANGUAGES.iter().find(|(key, _)| *key == extension).map(|(_, value)| *value)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn flags_lockfiles_and_generated_output_as_noise() {
    for path in [
      "pnpm-lock.yaml",
      "apps/web/package-lock.json",
      "go.sum",
      "dist/bundle.js",
      "src/__snapshots__/a.snap",
      "src/vendor/lib.js",
      "a/b/thing.generated.ts",
      "web/app.min.js",
    ] {
      assert!(is_noise(path), "{path}");
    }
  }

  #[test]
  fn leaves_ordinary_source_alone() {
    for path in ["src/index.ts", "lib/dist-helper.ts", "README.md", "src/build-config.ts"] {
      assert!(!is_noise(path), "{path}");
    }
  }

  #[test]
  fn maps_extensions_to_languages() {
    assert_eq!(language_of("src/a.ts"), Some("typescript"));
    assert_eq!(language_of("src/a.TSX"), Some("tsx"));
    assert_eq!(language_of("deploy/Dockerfile"), Some("dockerfile"));
    assert_eq!(language_of("deploy/Dockerfile.prod"), Some("dockerfile"));
    assert_eq!(language_of("Makefile"), None);
    assert_eq!(language_of("a/b/thing.unknownext"), None);
  }
}
