import assert from 'node:assert/strict';
import { test } from 'node:test';

import { waitForSubmit, type Ending, type SubmitWait } from './wait-for-submit.ts';

const TICK = 0.01;

/**
 * The timers inside are unref'd — in the CLI the listening server holds the
 * event loop open, so a test has to stand in for it or the process just exits.
 */
async function settle(wait: SubmitWait): Promise<Ending> {
  const keepAlive = setInterval(() => undefined, 5);
  try {
    return await wait.promise;
  } finally {
    clearInterval(keepAlive);
  }
}

test('resolves as submitted when the submit response flushes', async () => {
  const wait = waitForSubmit(0);
  wait.onSubmitStart();
  wait.onSubmit();
  assert.equal(await settle(wait), 'submitted');
});

test('resolves as a timeout when nothing is in flight', async () => {
  assert.equal(await settle(waitForSubmit(TICK)), 'timeout');
});

test('a submit in flight beats the timeout that fires under it', async () => {
  // The bug: the timeout tore the server down mid-response, so the browser
  // reported a failure for a review that had already been persisted.
  const wait = waitForSubmit(TICK, 1000);
  wait.onSubmitStart();
  await new Promise((resolve) => setTimeout(resolve, 30));
  wait.onSubmit();
  assert.equal(await settle(wait), 'submitted');
});

test('a submit that never finishes only delays the timeout by the grace', async () => {
  const wait = waitForSubmit(TICK, 10);
  wait.onSubmitStart();
  assert.equal(await settle(wait), 'timeout');
});

test('resolves as an interrupt on SIGINT', async () => {
  const wait = waitForSubmit(0);
  process.emit('SIGINT');
  assert.equal(await settle(wait), 'interrupt');
});
