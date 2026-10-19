/**
 * One card per service.
 *
 * Layout follows the macOS app: a header row (service avatar, name, state, and
 * the per-result actions), an optional warning strip, then the body — a word
 * result, a translation, an error, or a placeholder while the service is still
 * thinking — and a footer with the secondary actions.
 */

import { useEffect, useState } from "react";

import * as api from "../api";
import type { QueryRecord, QueryResult } from "../types";
import { ErrorView } from "./ErrorView";
import { TranslatedTextView } from "./TranslatedTextView";
import { WordResultView, type SpeakOptions } from "./WordResultView";
import {
  CopyIcon,
  ExternalIcon,
  LinkIcon,
  SpeakerIcon,
  StarIcon,
  StopIcon,
} from "./icons";
import {
  googleSearchUrl,
  isCopyable,
  isErrorVisible,
  serviceAccent,
  serviceInitial,
  showStopButton,
  translatedTextOf,
} from "./resultUtils";

export interface ResultCardProps {
  result: QueryResult;
  /** The queried text, used for the TTS fallback and the highlight. */
  queryText: string;
  /** Resolved Markdown decision: the per-result override wins over the setting. */
  markdownEnabled: boolean;
  /** The service that produced the most recent update, briefly outlined. */
  highlighted: boolean;
  onCopy: (text: string, message?: string) => void;
  onToast: (message: string) => void;
  onStop: () => void;
  onRetry: () => void;
  onPlay: (result: QueryResult) => void;
  onSpeak: (text: string, options?: SpeakOptions) => void;
  onQueryWord: (text: string) => void;
}

/** Tracks whether the current query is already in the favourites list. */
function useFavorite(result: QueryResult, onToast: (message: string) => void) {
  const [favorite, setFavorite] = useState(false);
  const [busy, setBusy] = useState(false);

  const record: QueryRecord = {
    queryText: result.queryText,
    fromLanguage: result.from,
    toLanguage: result.to,
    timestamp: Date.now(),
    translatedText: translatedTextOf(result),
  };

  useEffect(() => {
    let active = true;
    void api
      .isFavorite({
        queryText: result.queryText,
        fromLanguage: result.from,
        toLanguage: result.to,
        timestamp: 0,
      })
      .then((value) => {
        if (active) {
          setFavorite(value);
        }
      })
      .catch(() => {
        // A missing store is not worth surfacing; the button still works.
      });
    return () => {
      active = false;
    };
  }, [result.queryText, result.from, result.to]);

  const toggle = async () => {
    if (busy) {
      return;
    }
    setBusy(true);
    try {
      if (favorite) {
        await api.removeFavorite(record);
        setFavorite(false);
        onToast("已取消收藏");
      } else {
        await api.addFavorite(record);
        setFavorite(true);
        onToast("已收藏");
      }
    } catch (error) {
      onToast(typeof error === "string" ? error : "收藏失败");
    } finally {
      setBusy(false);
    }
  };

  return { favorite, toggle };
}

export function ResultCard({
  result,
  queryText,
  markdownEnabled,
  highlighted,
  onCopy,
  onToast,
  onStop,
  onRetry,
  onPlay,
  onSpeak,
  onQueryWord,
}: ResultCardProps) {
  const { favorite, toggle } = useFavorite(result, onToast);
  const copyable = isCopyable(result);
  const stopVisible = showStopButton(result);
  const errorVisible = isErrorVisible(result);
  const validation = result.validationMessage?.trim() ?? "";
  const hasBody =
    errorVisible ||
    Boolean(result.wordResult) ||
    Boolean(result.translatedResults?.length) ||
    Boolean(result.htmlString);

  const serviceName = result.serviceName || result.serviceTypeWithUniqueIdentifier;
  const accent = serviceAccent(result.serviceTypeWithUniqueIdentifier);

  return (
    <section
      className={`ez-result-card${highlighted ? " ez-result-card-active" : ""}`}
      aria-label={serviceName}
    >
      <header className="ez-card-header">
        <span className="ez-service-icon" style={accent} aria-hidden="true">
          {serviceInitial(serviceName)}
        </span>

        {result.link ? (
          <button
            type="button"
            className="ez-service-name ez-service-link"
            title={`在浏览器中打开 ${serviceName}`}
            onClick={() => void api.openLink(result.link ?? "")}
          >
            {serviceName}
            <LinkIcon size={12} />
          </button>
        ) : (
          <span className="ez-service-name">{serviceName}</span>
        )}

        {result.isStream ? <span className="ez-badge">流式</span> : null}

        <span className="ez-card-actions">
          {result.isLoading ? <span className="ez-spinner" aria-hidden="true" /> : null}

          {stopVisible ? (
            <button
              type="button"
              className="ez-icon-btn"
              title="停止"
              aria-label="停止"
              onClick={onStop}
            >
              <StopIcon size={13} />
            </button>
          ) : null}

          <button
            type="button"
            className="ez-icon-btn"
            title="发音"
            aria-label="发音"
            onClick={() => onPlay(result)}
          >
            <SpeakerIcon size={14} />
          </button>

          <button
            type="button"
            className="ez-icon-btn"
            title="复制"
            aria-label="复制"
            disabled={!copyable}
            onClick={() => onCopy(translatedTextOf(result), "已复制")}
          >
            <CopyIcon size={14} />
          </button>
        </span>
      </header>

      {validation.length > 0 ? (
        <p className="ez-validation ez-selectable">{validation}</p>
      ) : null}

      <div className="ez-card-body">
        {errorVisible && result.error ? (
          <ErrorView error={result.error} onRetry={onRetry} />
        ) : result.wordResult ? (
          <WordResultView
            word={result.wordResult}
            queryText={queryText}
            onSpeak={onSpeak}
            onQueryWord={onQueryWord}
          />
        ) : (
          <TranslatedTextView
            lines={result.translatedResults}
            markdownEnabled={markdownEnabled}
            htmlString={result.htmlString}
            queryText={queryText}
          />
        )}

        {!hasBody ? (
          <p className="ez-card-placeholder">
            {result.isLoading ? "正在查询…" : "没有结果"}
          </p>
        ) : null}
      </div>

      {hasBody && !errorVisible ? (
        <footer className="ez-card-footer">
          {result.showReplaceButton ? (
            <button
              type="button"
              className="ez-btn"
              title="用译文替换选中的文本"
              onClick={() => {
                void api
                  .replaceSelectedText(translatedTextOf(result))
                  .then(() => onToast("已替换"))
                  .catch((error: unknown) =>
                    onToast(typeof error === "string" ? error : "替换失败"),
                  );
              }}
            >
              替换
            </button>
          ) : null}

          <button
            type="button"
            className="ez-btn"
            title="在 Google 中搜索"
            onClick={() =>
              void api.openLink(googleSearchUrl(result.queryText || queryText))
            }
          >
            <ExternalIcon size={13} />
            Google 搜索
          </button>

          <button
            type="button"
            className="ez-btn"
            aria-pressed={favorite}
            title={favorite ? "取消收藏" : "收藏"}
            onClick={() => void toggle()}
          >
            <StarIcon size={13} />
            {favorite ? "已收藏" : "收藏"}
          </button>
        </footer>
      ) : null}
    </section>
  );
}
