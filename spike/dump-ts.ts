// Reference output for the parity harness: the TypeScript pipeline's hunks.json
// for a given scope, so the Rust spike can be diffed against it.
//
//   node --experimental-strip-types spike/dump-ts.ts <repo> [diff args...]

import { assembleDiff } from '../src/assemble.ts';

const [root, ...diffArgs] = process.argv.slice(2);
if (root === undefined) throw new Error('usage: dump-ts.ts <repo> [diff args...]');

process.stdout.write(`${JSON.stringify({ files: assembleDiff(root, diffArgs) }, null, 2)}\n`);
