import { createReadStream } from "node:fs";

export const DEFAULT_MAX_JSONL_LINE_BYTES = 4 * 1024 * 1024;

export interface JsonlLine {
  line: string;
  startOffset: number;
  nextOffset: number;
}

export interface JsonlReadDiagnostic {
  code: "line_too_large";
  startOffset: number;
  nextOffset: number;
  discardedBytes: number;
  newlineTerminated: boolean;
}

export interface JsonlReadOptions {
  maxLineBytes?: number;
  onDiagnostic?: (diagnostic: JsonlReadDiagnostic) => void;
}

export async function* readCompleteJsonlLines(
  filePath: string,
  startOffset = 0,
  options: JsonlReadOptions = {},
): AsyncGenerator<JsonlLine> {
  if (!Number.isSafeInteger(startOffset) || startOffset < 0) {
    throw new RangeError("startOffset must be a non-negative safe integer");
  }

  const maxLineBytes = options.maxLineBytes ?? DEFAULT_MAX_JSONL_LINE_BYTES;
  if (!Number.isSafeInteger(maxLineBytes) || maxLineBytes < 1) {
    throw new RangeError("maxLineBytes must be a positive safe integer");
  }

  const stream = createReadStream(filePath, { start: startOffset });
  let fragments: Buffer[] = [];
  let bufferedBytes = 0;
  let lineStartOffset = startOffset;
  let streamOffset = startOffset;
  let discarding = false;
  let discardedBytes = 0;

  for await (const chunk of stream) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    let cursor = 0;

    while (cursor < buffer.length) {
      const newlineIndex = buffer.indexOf(0x0a, cursor);
      const segmentEnd = newlineIndex >= 0 ? newlineIndex : buffer.length;
      const segment = buffer.subarray(cursor, segmentEnd);

      if (discarding) {
        discardedBytes += segment.length;
      } else if (bufferedBytes + segment.length > maxLineBytes) {
        discarding = true;
        discardedBytes = bufferedBytes + segment.length;
        fragments = [];
        bufferedBytes = 0;
      } else if (segment.length > 0) {
        fragments.push(segment);
        bufferedBytes += segment.length;
      }

      if (newlineIndex < 0) {
        break;
      }

      const nextOffset = streamOffset + newlineIndex + 1;
      if (discarding) {
        options.onDiagnostic?.({
          code: "line_too_large",
          startOffset: lineStartOffset,
          nextOffset,
          discardedBytes,
          newlineTerminated: true,
        });
      } else if (bufferedBytes > 0) {
        let lineBuffer =
          fragments.length === 1
            ? fragments[0]
            : Buffer.concat(fragments, bufferedBytes);
        if (lineBuffer.at(-1) === 0x0d) {
          lineBuffer = lineBuffer.subarray(0, lineBuffer.length - 1);
        }
        if (lineBuffer.length > 0) {
          yield {
            line: lineBuffer.toString("utf8"),
            startOffset: lineStartOffset,
            nextOffset,
          };
        }
      }

      fragments = [];
      bufferedBytes = 0;
      discarding = false;
      discardedBytes = 0;
      lineStartOffset = nextOffset;
      cursor = newlineIndex + 1;
    }

    streamOffset += buffer.length;
  }

  if (discarding) {
    // An unterminated record is never checkpoint-safe. Report the observed
    // bytes, but point nextOffset at the start so a later append is retried.
    options.onDiagnostic?.({
      code: "line_too_large",
      startOffset: lineStartOffset,
      nextOffset: lineStartOffset,
      discardedBytes,
      newlineTerminated: false,
    });
  }
}
