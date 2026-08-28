// Reference output for the parity harness: the TypeScript pipeline's hunks.json
// for a given scope, so the Rust port can be diffed against it.
//
//   node --experimental-strip-types spike/dump-ts.ts <repo> [diff args...]
//                                                    [-- <pathspec>...]

import { assembleDiff } from '../src/assemble.ts';

const [root, ...rest] = process.argv.slice(2);
if (root === undefined) throw new Error('usage: dump-ts.ts <repo> [diff args...]');

const separator = rest.indexOf('--');
const diffArgs = separator === -1 ? rest : rest.slice(0, separator);
const paths = separator === -1 ? [] : rest.slice(separator + 1);

process.stdout.write(
  `${JSON.stringify({ files: assembleDiff(root, diffArgs, paths) }, null, 2)}\n`,
);
