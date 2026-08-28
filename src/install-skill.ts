import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

/** Ships in the package next to dist/ and ui/; see "files" in package.json. */
export const SKILL_SOURCE = resolve(import.meta.dirname, '..', 'skills', 'diffpane', 'SKILL.md');

export function defaultSkillDir(): string {
  return join(homedir(), '.claude', 'skills');
}

export interface SkillInstall {
  path: string;
  replaced: boolean;
}

/**
 * Overwrites by design: the skill is versioned with the binary that writes it,
 * so a stale copy is the failure mode worth preventing. Reports which happened
 * so a hand-edited skill does not vanish without a word.
 */
export function installSkill(skillDir: string = defaultSkillDir()): SkillInstall {
  if (!existsSync(SKILL_SOURCE)) {
    throw new Error(`packaged skill is missing: ${SKILL_SOURCE}`);
  }
  const target = join(skillDir, 'diffpane');
  const path = join(target, 'SKILL.md');
  const replaced = existsSync(path);
  mkdirSync(target, { recursive: true });
  copyFileSync(SKILL_SOURCE, path);
  return { path, replaced };
}
