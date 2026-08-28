export type Ending = 'submitted' | 'timeout' | 'interrupt';

export interface SubmitWait {
  promise: Promise<Ending>;
  /** Called when a submit request arrives, before its response is written. */
  onSubmitStart: () => void;
  /** Called once the submit response has flushed. */
  onSubmit: () => void;
}

/** How long a submit already in flight may hold up a timeout or a Ctrl+C. */
const GRACE_MS = 2000;

/**
 * Resolves when the review is submitted, the timeout fires, or the user quits.
 *
 * A submit already in flight wins the race: the caller tears the server down on
 * resolution, and destroying the socket under a submit makes the browser report
 * a failure for a review that was already written to disk. The grace bounds
 * that wait, so a response that never lands cannot hang the CLI.
 */
export function waitForSubmit(timeoutSeconds: number, graceMs = GRACE_MS): SubmitWait {
  let onSubmit = (): void => undefined;
  let onSigint = (): void => undefined;
  let isSubmitting = false;
  const promise = new Promise<Ending>((resolveWith) => {
    onSubmit = (): void => {
      resolveWith('submitted');
    };
    const end = (ending: Ending): void => {
      if (!isSubmitting) {
        resolveWith(ending);
        return;
      }
      setTimeout(() => {
        resolveWith(ending);
      }, graceMs).unref();
    };
    onSigint = (): void => {
      process.stderr.write('\n');
      end('interrupt');
    };
    process.once('SIGINT', onSigint);
    if (timeoutSeconds > 0) {
      setTimeout(() => {
        end('timeout');
      }, timeoutSeconds * 1000).unref();
    }
  });
  return {
    promise: promise.finally(() => {
      process.removeListener('SIGINT', onSigint);
    }),
    onSubmitStart: (): void => {
      isSubmitting = true;
    },
    onSubmit: (): void => {
      onSubmit();
    },
  };
}
