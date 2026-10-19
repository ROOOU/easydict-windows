/**
 * The translation body of a card.
 *
 * A service returns `translatedResults` as an array of paragraphs. With
 * Markdown rendering on (the per-result `markdownRenderingOverride` wins over
 * the global setting) the paragraphs are handed to the local Markdown renderer;
 * with it off they are shown verbatim — no syntax is interpreted, which is what
 * the macOS app does for "plain text" mode.
 */

import { Markdown } from "./markdown";
import { highlightText, htmlToTextLines } from "./resultUtils";

export interface TranslatedTextViewProps {
  /** `QueryResult.translatedResults`; `null` means "nothing yet". */
  lines: string[] | null;
  markdownEnabled: boolean;
  /** Local dictionary HTML, used when there are no translated paragraphs. */
  htmlString?: string | null;
  /** The queried word, marked wherever it appears in the translation. */
  queryText?: string | null;
}

export function TranslatedTextView({
  lines,
  markdownEnabled,
  htmlString,
  queryText,
}: TranslatedTextViewProps) {
  const paragraphs = (lines ?? [])
    .map((line) => line.replace(/\s+$/g, ""))
    .filter((line) => line.length > 0);

  if (paragraphs.length === 0 && !htmlString) {
    // Nothing yet: the card shows its own "no result" placeholder instead.
    return null;
  }

  if (paragraphs.length === 0 && htmlString) {
    const dictionaryLines = htmlToTextLines(htmlString);
    return (
      <div className="ez-translated ez-translated-dictionary">
        {dictionaryLines.map((line, index) => (
          <p className="ez-translated-line" key={`dictionary-${index}`}>
            {highlightText(line, queryText)}
          </p>
        ))}
      </div>
    );
  }

  if (markdownEnabled) {
    return (
      <div className="ez-translated">
        <Markdown text={paragraphs.join("\n\n")} highlight={queryText} />
      </div>
    );
  }

  return (
    <div className="ez-translated ez-translated-plain">
      {paragraphs.map((paragraph, index) => (
        <p className="ez-translated-line" key={`line-${index}`}>
          {highlightText(paragraph, queryText)}
        </p>
      ))}
    </div>
  );
}
