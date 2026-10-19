/**
 * The scrolling stack of result cards.
 *
 * The outer element scrolls; the inner one is content-sized and is what the
 * auto-fit logic measures, because a scroll container reports its viewport as
 * its height while the inner wrapper always reports what the content needs.
 */

import type { CSSProperties, RefObject } from "react";

import type { QueryResult } from "../types";
import { ResultCard } from "./ResultCard";
import type { SpeakOptions } from "./WordResultView";

export interface ResultListProps {
  results: QueryResult[];
  queryText: string;
  /** `useFontRatio()`: the result text scale, applied as a CSS variable. */
  fontRatio: number;
  /** The global `enableMarkdownRendering`, before per-result overrides. */
  markdownEnabled: boolean;
  lastIdentifier: string | null;
  running: boolean;
  /** Attached to the content-sized inner wrapper, for the height measurement. */
  contentRef: RefObject<HTMLDivElement | null>;
  onCopy: (text: string, message?: string) => void;
  onToast: (message: string) => void;
  onStop: () => void;
  onRetry: () => void;
  onPlay: (result: QueryResult) => void;
  onSpeak: (text: string, options?: SpeakOptions) => void;
  onQueryWord: (text: string) => void;
}

export function ResultList({
  results,
  queryText,
  fontRatio,
  markdownEnabled,
  lastIdentifier,
  running,
  contentRef,
  onCopy,
  onToast,
  onStop,
  onRetry,
  onPlay,
  onSpeak,
  onQueryWord,
}: ResultListProps) {
  const style = { "--ez-font-ratio": String(fontRatio) } as CSSProperties;

  return (
    <div className="ez-results ez-scroll" style={style}>
      <div className="ez-results-inner" ref={contentRef}>
        {results.length === 0 ? (
          <div className="ez-results-empty">
            <span className="ez-results-empty-title">
              {running ? "正在查询…" : "输入翻译"}
            </span>
            <span className="ez-results-empty-hint">
              {running
                ? "等待服务返回结果"
                : "输入文字后按 Enter 翻译，Shift + Enter 换行"}
            </span>
          </div>
        ) : (
          results.map((result) => (
            <ResultCard
              key={result.serviceTypeWithUniqueIdentifier}
              result={result}
              queryText={queryText}
              markdownEnabled={result.markdownRenderingOverride ?? markdownEnabled}
              highlighted={result.serviceTypeWithUniqueIdentifier === lastIdentifier}
              onCopy={onCopy}
              onToast={onToast}
              onStop={onStop}
              onRetry={onRetry}
              onPlay={onPlay}
              onSpeak={onSpeak}
              onQueryWord={onQueryWord}
            />
          ))
        )}
      </div>
    </div>
  );
}
