// Symbols the docs corpus mentions. Names are deliberately distinctive so the
// docs pass's conservative resolver can match them by unique bare name (none
// is in `docs::STOP_WORDS`, and none is declared twice).

export interface RetryPolicy {
  attempts: number;
}

export class TelemetrySink {
  emit(event: string): void {
    console.log(event);
  }
}

export function makeRetryPolicy(attempts: number): RetryPolicy {
  return { attempts };
}

export function drainTelemetry(sink: TelemetrySink): void {
  sink.emit("drain");
}
