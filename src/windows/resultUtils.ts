/**
 * Small pure helpers shared by the query window's components.
 *
 * Everything here mirrors a rule that already exists on the Rust side
 * (`util/analysis.rs`, `model/result.rs`) so the two halves agree on what
 * counts as a word, what a "translated text" is, and which results are
 * copyable.
 */

import { createElement, Fragment, type ReactNode } from "react";

import type { QueryResult, TranslateWordResult } from "../types";

const ENGLISH_WORD_MAX_LENGTH = 20;

function stripQuotes(text: string): string {
  return text.replace(/^["'“”‘’]+|["'“”‘’]+$/g, "").trim();
}

/**
 * `util::analysis::is_english_word`: quotes removed, at most 20 characters,
 * ASCII letters only. Decides whether a TTS request may take the "English word"
 * path (Youdao's dictionary voice).
 */
export function isEnglishWord(text: string): boolean {
  const trimmed = stripQuotes(text.trim());
  return (
    trimmed.length > 0 &&
    trimmed.length <= ENGLISH_WORD_MAX_LENGTH &&
    /^[a-zA-Z]+$/.test(trimmed)
  );
}

/** The word-result means, flattened the way the app copies them. */
function wordMeansText(word: TranslateWordResult): string {
  const parts = word.parts ?? [];
  return parts
    .map((part) => {
      const means = part.means.join("; ");
      return part.part ? `${part.part} ${means}` : means;
    })
    .filter((line) => line.trim().length > 0)
    .join("\n");
}

/**
 * The text a card's copy button should put on the clipboard: the translated
 * paragraphs when the service produced them, otherwise a word result's means.
 */
export function translatedTextOf(result: QueryResult): string {
  const lines = (result.translatedResults ?? [])
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  if (lines.length > 0) {
    return lines.join("\n");
  }
  if (result.wordResult) {
    return wordMeansText(result.wordResult);
  }
  return "";
}

/** A word result with no translated paragraphs still counts as copyable. */
export function isCopyable(result: QueryResult): boolean {
  return translatedTextOf(result).length > 0;
}

/**
 * Whether the card should render its error body. `noResult` is the backend's
 * "this service had nothing useful" marker: it is only shown once the user asks
 * for it (`manualShow`), and `isShowing` already reflects that.
 */
export function isErrorVisible(result: QueryResult): boolean {
  const error = result.error;
  if (!error) {
    return false;
  }
  if (error.type === "noResult" && !result.manualShow) {
    return false;
  }
  return true;
}

/**
 * The stop button belongs to streaming services only, and it stays up until the
 * stream reports that it finished — a stream that stopped early (an error, or a
 * cancelled request) keeps offering the stop affordance so the user can abort.
 */
export function showStopButton(result: QueryResult): boolean {
  if (!result.isStream) {
    return false;
  }
  return (result.isLoading && result.isStream) || !result.isStreamFinished;
}

/** Turns a thrown value into something safe to put in a toast. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "未知错误";
}

/**
 * Local dictionary results (MDict) arrive as an HTML string. The renderer must
 * not inject markup — `dangerouslySetInnerHTML` is banned for anything a model
 * or a remote service can influence — so the HTML is parsed off-document with
 * `DOMParser` and reduced to plain text lines. Links and styling from the
 * dictionary are lost, which is the price of not injecting it.
 */
export function htmlToTextLines(html: string): string[] {
  if (typeof DOMParser === "undefined") {
    return html.split("\n");
  }
  const document_ = new DOMParser().parseFromString(html, "text/html");
  for (const node of Array.from(document_.querySelectorAll("script, style"))) {
    node.remove();
  }
  for (const br of Array.from(document_.querySelectorAll("br"))) {
    br.replaceWith("\n");
  }
  for (const block of Array.from(
    document_.querySelectorAll("p, div, li, tr, h1, h2, h3, h4, h5, h6"),
  )) {
    block.append("\n");
  }
  const text = document_.body.textContent ?? "";
  return text
    .split("\n")
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter((line) => line.length > 0);
}

/** A stable colour per service, so the header avatars stay recognisable. */
export function serviceAccent(identifier: string): { background: string; color: string } {
  let hash = 0;
  for (let index = 0; index < identifier.length; index += 1) {
    hash = (hash * 31 + identifier.charCodeAt(index)) % 360;
  }
  return {
    background: `hsl(${hash} 62% 52% / 0.18)`,
    color: `hsl(${hash} 64% 42%)`,
  };
}

/** The letter drawn in the service avatar. */
export function serviceInitial(name: string): string {
  const trimmed = name.trim();
  return trimmed.length > 0 ? trimmed.slice(0, 1) : "?";
}

/**
 * One of the three "open in …" links the title bar builds by percent-encoding
 * the query text into a scheme (`EZOpenLinkButton.h`): `EZGoogleWebSearchURL`.
 */
export function googleSearchUrl(text: string): string {
  return `https://www.google.com/search?q=${encodeURIComponent(text)}`;
}

/** `EZEudicAppURLScheme`. */
export function eudicUrl(text: string): string {
  return `eudic://dict/${encodeURIComponent(text)}`;
}

/** `EZAppleDictionaryAppURLScheme`. */
export function appleDictionaryUrl(text: string): string {
  return `dict://${encodeURIComponent(text)}`;
}

/**
 * Wraps every occurrence of `needle` in `<mark>`, the way the macOS app
 * highlights the queried word inside a translation. Built out of React
 * elements, never markup, so the source text stays inert.
 */
export function highlightText(text: string, needle: string | null | undefined): ReactNode {
  const query = needle?.trim() ?? "";
  if (!query || query.length > 40) {
    return text;
  }
  const lowerText = text.toLocaleLowerCase();
  const lowerQuery = query.toLocaleLowerCase();
  const nodes: ReactNode[] = [];
  let cursor = 0;
  let key = 0;
  for (;;) {
    const found = lowerText.indexOf(lowerQuery, cursor);
    if (found === -1) {
      break;
    }
    if (found > cursor) {
      nodes.push(text.slice(cursor, found));
    }
    nodes.push(
      createElement(
        "mark",
        { key: `mark-${key}`, className: "ez-mark" },
        text.slice(found, found + query.length),
      ),
    );
    key += 1;
    cursor = found + query.length;
  }
  if (nodes.length === 0) {
    return text;
  }
  if (cursor < text.length) {
    nodes.push(text.slice(cursor));
  }
  return createElement(Fragment, null, ...nodes);
}
